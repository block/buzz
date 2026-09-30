import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/crypto/nip44.dart';
import '../../shared/relay/relay.dart';

/// NIP-ER event reminder kind — matches desktop's `KIND_EVENT_REMINDER`.
const kindEventReminder = 30300;

/// Message a reminder points at. Mirrors desktop's `ReminderTarget`.
@immutable
class ReminderTarget {
  final String eventId;
  final String channelId;
  final String preview;
  final String authorPubkey;

  const ReminderTarget({
    required this.eventId,
    required this.channelId,
    required this.preview,
    required this.authorPubkey,
  });
}

/// A decrypted NIP-ER reminder. Mirrors desktop's `Reminder`.
@immutable
class Reminder {
  /// The `d`-tag (unique reminder identity).
  final String id;

  /// Unix seconds when due; null for done/cancelled reminders.
  final int? notBefore;

  final String status; // "pending" | "done" | "cancelled"
  final ReminderTarget? target;
  final String? note;
  final int createdAt;
  final String eventId;

  const Reminder({
    required this.id,
    required this.notBefore,
    required this.status,
    required this.target,
    required this.note,
    required this.createdAt,
    required this.eventId,
  });

  bool isDue(int nowUnixSeconds) =>
      status == 'pending' && notBefore != null && notBefore! <= nowUnixSeconds;
}

/// Parse NIP-ER `not_before`: ASCII digits, no leading zero except "0".
/// Mirrors desktop's `parseNotBefore` strictness.
int? parseNotBefore(String raw) {
  if (!RegExp(r'^(0|[1-9][0-9]*)$').hasMatch(raw)) return null;
  return int.tryParse(raw);
}

/// Validate decrypted reminder plaintext — desktop's `parseReminderContent`.
/// Off-shape content fails closed (returns null).
({String status, ReminderTarget? target, String? note})? parseReminderContent(
  String plaintext,
) {
  final Object? parsed;
  try {
    parsed = jsonDecode(plaintext);
  } catch (_) {
    return null;
  }
  if (parsed is! Map<String, dynamic>) return null;

  final status = parsed['status'];
  if (status != 'pending' && status != 'done' && status != 'cancelled') {
    return null;
  }
  final note = parsed['note'];
  if (note != null && note is! String) return null;

  ReminderTarget? target;
  final rawTarget = parsed['target'];
  if (rawTarget != null) {
    if (rawTarget is! Map<String, dynamic>) return null;
    final eventId = rawTarget['eventId'];
    final channelId = rawTarget['channelId'];
    final preview = rawTarget['preview'];
    final authorPubkey = rawTarget['authorPubkey'];
    if (eventId is! String ||
        channelId is! String ||
        preview is! String ||
        authorPubkey is! String) {
      return null;
    }
    target = ReminderTarget(
      eventId: eventId,
      channelId: channelId,
      preview: preview,
      authorPubkey: authorPubkey,
    );
  }

  final noteText = note as String?;
  if (target == null && (noteText == null || noteText.isEmpty)) return null;

  return (status: status as String, target: target, note: noteText);
}

/// Decode one kind:30300 event into a [Reminder], or null when malformed.
Reminder? decodeReminderEvent(
  NostrEvent event, {
  required String Function(String ciphertext) decrypt,
}) {
  String? dTag;
  int? notBefore;
  for (final tag in event.tags) {
    if (tag.length >= 2 && tag[0] == 'd') dTag ??= tag[1];
    if (tag.length >= 2 && tag[0] == 'not_before') {
      notBefore ??= parseNotBefore(tag[1]);
    }
  }
  if (dTag == null) return null;

  final String plaintext;
  try {
    plaintext = decrypt(event.content);
  } catch (_) {
    return null;
  }
  final content = parseReminderContent(plaintext);
  if (content == null) return null;

  return Reminder(
    id: dTag,
    notBefore: notBefore,
    status: content.status,
    target: content.target,
    note: content.note,
    createdAt: event.createdAt,
    eventId: event.id,
  );
}

/// Fetches the user's NIP-ER reminders (kind 30300, self-encrypted). These
/// are the same relay events Buzz Desktop reads and writes, so reminders
/// created on Desktop appear here and vice versa.
class RemindersNotifier extends AsyncNotifier<List<Reminder>> {
  @override
  Future<List<Reminder>> build() {
    ref.watch(relayConfigProvider);
    ref.watch(relaySessionProvider);
    return _fetch();
  }

  Future<List<Reminder>> _fetch() async {
    final config = ref.read(relayConfigProvider);
    final myPk = ref.read(myPubkeyProvider);
    final nsec = config.nsec?.trim();
    if (myPk == null || nsec == null || nsec.isEmpty) return const [];

    final String privkeyHex;
    try {
      privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    } catch (_) {
      return const [];
    }
    if (privkeyHex.isEmpty) return const [];
    final conversationKey = getConversationKey(privkeyHex, myPk);

    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(
      NostrFilter(
        kinds: const [kindEventReminder],
        authors: [myPk],
        limit: 200,
      ),
    );

    // Parameterized-replaceable: keep only the newest event per d-tag.
    final newestByDTag = <String, NostrEvent>{};
    for (final event in events) {
      final dTag = event.getTagValue('d');
      if (dTag == null) continue;
      final existing = newestByDTag[dTag];
      if (existing == null || event.createdAt > existing.createdAt) {
        newestByDTag[dTag] = event;
      }
    }

    final reminders = <Reminder>[];
    for (final event in newestByDTag.values) {
      final reminder = decodeReminderEvent(
        event,
        decrypt: (ciphertext) => nip44Decrypt(conversationKey, ciphertext),
      );
      if (reminder != null) reminders.add(reminder);
    }
    reminders.sort(
      (a, b) =>
          (a.notBefore ?? a.createdAt).compareTo(b.notBefore ?? b.createdAt),
    );
    return reminders;
  }

  Future<void> refresh() async {
    state = await AsyncValue.guard(_fetch);
  }
}

final remindersProvider =
    AsyncNotifierProvider<RemindersNotifier, List<Reminder>>(
      RemindersNotifier.new,
    );

/// Count of due pending reminders — powers the filter badge.
final dueReminderCountProvider = Provider<int>((ref) {
  final reminders = ref.watch(remindersProvider).value ?? const <Reminder>[];
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  return reminders.where((r) => r.isDue(now)).length;
});
