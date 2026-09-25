import 'package:flutter/foundation.dart';

import '../utils/string_utils.dart';

/// Pinned version of the portable identity-name contract implemented here.
///
/// Spec and fixtures:
/// https://github.com/block/buzz-app/blob/92c2fb2/src/bundled/identity-naming/README.md
const identityNamePolicyVersion = 1;

/// One naming fact for an identity. [pubkey] and [ownerPubkey] must be valid
/// 64-character hex public keys; callers choose names and fallbacks first.
@immutable
class NamingIdentity {
  final String pubkey;
  final String name;
  final bool isAgent;
  final String? ownerPubkey;

  const NamingIdentity({
    required this.pubkey,
    required this.name,
    this.isAgent = false,
    this.ownerPubkey,
  });

  @override
  bool operator ==(Object other) =>
      other is NamingIdentity &&
      other.pubkey == pubkey &&
      other.name == name &&
      other.isAgent == isAgent &&
      other.ownerPubkey == ownerPubkey;

  @override
  int get hashCode => Object.hash(pubkey, name, isAgent, ownerPubkey);
}

/// A resolved display label. [qualifier] is only the key suffix, when added.
@immutable
class ResolvedIdentityName {
  final String name;
  final String? qualifier;

  const ResolvedIdentityName(this.name, [this.qualifier]);

  @override
  bool operator ==(Object other) =>
      other is ResolvedIdentityName &&
      other.name == name &&
      other.qualifier == qualifier;

  @override
  int get hashCode => Object.hash(name, qualifier);

  @override
  String toString() => 'ResolvedIdentityName($name, $qualifier)';
}

// ECMAScript String.prototype.trim code points, as the contract requires.
// Dart's String.trim also removes U+0085, which the contract keeps.
bool _isContractWhitespace(int unit) =>
    (unit >= 0x09 && unit <= 0x0D) ||
    unit == 0x20 ||
    unit == 0xA0 ||
    unit == 0x1680 ||
    (unit >= 0x2000 && unit <= 0x200A) ||
    unit == 0x2028 ||
    unit == 0x2029 ||
    unit == 0x202F ||
    unit == 0x205F ||
    unit == 0x3000 ||
    unit == 0xFEFF;

/// Trims [value] exactly as the identity-name contract specifies.
String trimIdentityName(String value) {
  var start = 0;
  var end = value.length;
  while (start < end && _isContractWhitespace(value.codeUnitAt(start))) {
    start++;
  }
  while (end > start && _isContractWhitespace(value.codeUnitAt(end - 1))) {
    end--;
  }
  return value.substring(start, end);
}

class _Row {
  final String key;
  final NamingIdentity identity;
  final String original;
  final int priority;
  final bool mine;
  String base;
  String label;
  int length = 0;
  String? suffix;

  _Row({
    required this.key,
    required this.identity,
    required this.original,
    required this.priority,
    required this.mine,
  }) : base = original,
       label = original;
}

/// Resolves distinct display labels for the selected identities.
///
/// Display policy only: a readable owner or key suffix grants no authority.
/// Implements contextual identity names v1 (see [identityNamePolicyVersion]).
/// Omitting [candidates] selects every supplied fact; an empty set selects
/// none. Throws [ArgumentError] for an invalid public key.
Map<String, ResolvedIdentityName> resolveIdentityNames(
  List<NamingIdentity> identities, {
  String? viewer,
  Iterable<String>? candidates,
}) {
  final normalizedViewer = viewer?.toLowerCase();
  // Last fact per key: the preferred alias and the owner-lookup name.
  final preferred = <String, NamingIdentity>{
    for (final identity in identities) identity.pubkey.toLowerCase(): identity,
  };
  final selected = candidates?.map((key) => key.toLowerCase()).toSet();
  // One working row per (key, trimmed name); the last fact supplies metadata.
  final aliases = <(String, String), NamingIdentity>{};
  for (final identity in identities) {
    final key = (
      identity.pubkey.toLowerCase(),
      trimIdentityName(identity.name),
    );
    aliases[key] = identity;
  }
  final rows = <_Row>[
    for (final MapEntry(key: (key, name), value: identity) in aliases.entries)
      if (selected == null || selected.contains(key))
        () {
          final owner = identity.ownerPubkey?.toLowerCase();
          final mine =
              normalizedViewer != null &&
              (key == normalizedViewer || owner == normalizedViewer);
          return _Row(
            key: key,
            identity: identity,
            original: name,
            mine: mine,
            priority: identity.isAgent
                ? (mine ? 2 : 3)
                : (key == normalizedViewer ? 0 : 1),
          );
        }(),
  ];
  final npubs = <String, String>{};
  String npubFor(String key) => npubs[key] ??= () {
    final npub = fullNpub(key);
    if (npub == null || key.length != 64) {
      throw ArgumentError.value(key, 'pubkey', 'not a hex public key');
    }
    return npub;
  }();

  while (true) {
    final groups = <String, List<_Row>>{};
    for (final row in rows) {
      (groups[row.label] ??= []).add(row);
    }
    final collisions = [
      for (final group in groups.values)
        if (group.map((row) => row.key).toSet().length > 1) group,
    ];
    if (collisions.isEmpty) break;
    for (final group in collisions) {
      final best = group
          .map((row) => row.priority)
          .reduce((a, b) => a < b ? a : b);
      final winners = {
        for (final row in group)
          if (row.priority == best) row.key,
      };
      final changing = [
        for (final row in group)
          if (winners.length != 1 || !winners.contains(row.key)) row,
      ];
      final groupHasHuman = group.any((row) => !row.identity.isAgent);
      var qualified = false;
      for (final row in changing) {
        if (!row.identity.isAgent || row.length != 0) continue;
        final ownerKey = row.identity.ownerPubkey?.toLowerCase();
        final ownerFact = ownerKey == null ? null : preferred[ownerKey];
        final owner = ownerFact == null ? '' : trimIdentityName(ownerFact.name);
        final readable = !row.mine && owner.isNotEmpty
            ? '$owner\u2019s ${row.original}'
            : groupHasHuman
            ? '${row.original} (agent)'
            : row.base;
        if (readable != row.base) {
          row.base = readable;
          row.label = readable;
          qualified = true;
        }
      }
      if (qualified) continue;
      for (final row in changing) {
        row.length = row.length == 0 ? 4 : row.length + 1;
        final npub = npubFor(row.key);
        row.suffix = row.length <= npub.length
            ? npub.substring(npub.length - row.length)
            : '$npub \u00b7 ${row.length - npub.length}';
        row.label = '${row.base} \u00b7 ${row.suffix}';
      }
    }
  }

  return {
    for (final row in rows)
      if (row.original == trimIdentityName(preferred[row.key]!.name))
        row.key: ResolvedIdentityName(row.label, row.suffix),
  };
}
