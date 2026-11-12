import 'dart:convert';

import 'read_state_format.dart';

/// Packs frontiers without evicting a key or moving an existing carrier.
///
/// [previous] is the durably saved placement, including empty reusable slots.
/// The caller must persist the returned placement and its slot IDs before any
/// publication. Override groups and their frontiers belong in [primary].
/// A capacity error leaves the caller's previous placement untouched.
List<Map<String, int>> packReadStateContexts(
  Map<String, int> frontiers, {
  required String clientId,
  List<Map<String, int>> previous = const [],
  Map<String, int> primary = const {},
  int maxBytes = readStatePlaintextBytes,
}) {
  int size(Map<String, int> contexts) => utf8
      .encode(
        jsonEncode(
          ReadStateBlob(clientId: clientId, contexts: contexts).toJson(),
        ),
      )
      .length;
  bool fits(Map<String, int> contexts) =>
      contexts.length <= 10000 && size(contexts) <= maxBytes;
  final slots = <Map<String, int>>[
    for (final old in previous)
      {
        for (final key in old.keys)
          if (frontiers.containsKey(key) && !primary.containsKey(key))
            key: frontiers[key]!,
      },
  ];
  if (slots.isEmpty) slots.add({});
  slots.first.addAll(primary);
  for (final slot in slots) {
    if (!fits(slot)) {
      throw StateError('Read-state slot exceeds its plaintext budget');
    }
  }
  final assigned = {for (final slot in slots) ...slot.keys};
  final keys = frontiers.keys.where((key) => !assigned.contains(key)).toList()
    ..sort();
  final sizes = slots.map(size).toList();
  final emptySize = size({});
  // Account for the exact compact JSON entry and its optional comma without
  // repeatedly serializing every full slot for each incoming key.
  for (final key in keys) {
    final value = frontiers[key]!;
    final bytes = utf8.encode(jsonEncode(key)).length + 1 + '$value'.length;
    var placed = false;
    for (var index = 0; index < slots.length; index++) {
      final slot = slots[index];
      final cost = bytes + (slot.isEmpty ? 0 : 1);
      if (slot.length < 10000 && sizes[index] + cost <= maxBytes) {
        slot[key] = value;
        sizes[index] += cost;
        placed = true;
        break;
      }
    }
    if (!placed) {
      if (emptySize + bytes > maxBytes) {
        throw StateError('Read-state entry exceeds its plaintext budget');
      }
      slots.add({key: value});
      sizes.add(emptySize + bytes);
    }
  }
  return [
    for (final slot in slots)
      {for (final key in slot.keys.toList()..sort()) key: slot[key]!},
  ];
}
