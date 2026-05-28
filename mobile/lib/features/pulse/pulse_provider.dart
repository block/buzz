import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../profile/user_cache_provider.dart';
import 'pulse_models.dart';

final globalNotesProvider = FutureProvider<List<UserNote>>((ref) async {
  final session = ref.watch(relaySessionProvider.notifier);
  final events = await session.fetchHistory(NostrFilters.globalNotes());
  return _notesFromEvents(events);
});

final notesTimelineProvider = FutureProvider.family<List<UserNote>, String>((
  ref,
  pubkeysKey,
) async {
  final pubkeys = parsePulseKey(pubkeysKey);
  if (pubkeys.isEmpty) return const [];
  final session = ref.watch(relaySessionProvider.notifier);
  final events = await session.fetchHistory(
    NostrFilters.notesTimeline(pubkeys),
  );
  return _notesFromEvents(events);
});

final likedNotesProvider = FutureProvider<List<UserNote>>((ref) async {
  final pubkey = ref.watch(myPubkeyProvider);
  if (pubkey == null) return const [];
  final session = ref.watch(relaySessionProvider.notifier);
  final reactions = await session.fetchHistory(
    NostrFilters.userReactions(pubkey),
  );
  final liveReactions = await _filterDeletedReactions(
    session,
    reactions,
    deletionAuthors: [pubkey],
  );
  final ids = <String>[];
  final seen = <String>{};
  for (final reaction in liveReactions) {
    if (reaction.content != '+') continue;
    final noteId = _lastETag(reaction.tags);
    if (noteId != null && seen.add(noteId)) ids.add(noteId);
  }
  if (ids.isEmpty) return const [];
  final notes = await session.fetchHistory(NostrFilters.notesByIds(ids));
  return _notesFromEvents(notes);
});

final noteReactionsProvider =
    FutureProvider.family<Map<String, PulseReactionState>, String>((
      ref,
      noteIdsKey,
    ) async {
      final noteIds = parsePulseKey(noteIdsKey);
      if (noteIds.isEmpty) return const {};
      final currentPubkey = ref.watch(myPubkeyProvider)?.toLowerCase();
      final session = ref.watch(relaySessionProvider.notifier);
      final reactions = await session.fetchHistory(
        NostrFilters.noteReactions(noteIds),
      );
      final events = await _filterDeletedReactions(session, reactions);
      final pubkeysByNote = <String, Set<String>>{};
      final currentReactionIds = <String, String>{};

      for (final event in events) {
        if (event.content != '+') continue;
        final noteId = _lastETag(event.tags);
        if (noteId == null) continue;
        final pubkey = event.pubkey.toLowerCase();
        pubkeysByNote.putIfAbsent(noteId, () => <String>{}).add(pubkey);
        if (currentPubkey != null && pubkey == currentPubkey) {
          currentReactionIds[noteId] = event.id;
        }
      }

      return {
        for (final noteId in noteIds)
          noteId: PulseReactionState(
            count: pubkeysByNote[noteId]?.length ?? 0,
            reactedByCurrentUser: currentReactionIds.containsKey(noteId),
            currentUserReactionId: currentReactionIds[noteId],
          ),
      };
    });

final contactListProvider = FutureProvider.family<List<ContactEntry>, String>((
  ref,
  pubkey,
) async {
  if (pubkey.isEmpty) return const [];
  final session = ref.watch(relaySessionProvider.notifier);
  final events = await session.fetchHistory(
    NostrFilters.contactList(pubkey.toLowerCase()),
  );
  return contactsFromEvents(events);
});

final agentPubkeysProvider = FutureProvider<List<String>>((ref) async {
  final session = ref.watch(relaySessionProvider.notifier);

  // Primary source: kind:10100 agent profile events (each agent signs its own).
  final profileEvents = await session.fetchHistory(
    NostrFilters.agentProfiles(),
  );
  final pubkeys = <String>{};
  for (final event in profileEvents) {
    pubkeys.add(event.pubkey.toLowerCase());
    final p = event.getTagValue('p');
    if (p != null) pubkeys.add(p.toLowerCase());
  }

  // Fallback: relay membership list (kind:13534) — extract members with
  // role "bot". This catches managed agents that may not have published
  // a kind:10100 profile event yet.
  final memberEvents = await session.fetchHistory(NostrFilters.relayMembers());
  if (memberEvents.isNotEmpty) {
    final event = memberEvents.first;
    for (final tag in event.tags) {
      if (tag.length >= 3 && tag[0] == 'member' && tag[2] == 'bot') {
        pubkeys.add(tag[1].toLowerCase());
      }
      // NIP-29 fallback: ["p", pubkey, relay_url?, role?]
      if (tag.length >= 4 && tag[0] == 'p' && tag[3] == 'bot') {
        pubkeys.add(tag[1].toLowerCase());
      }
    }
  }

  return pubkeys.toList();
});

final agentNotesProvider = FutureProvider<List<UserNote>>((ref) async {
  final pubkeys = await ref.watch(agentPubkeysProvider.future);
  if (pubkeys.isEmpty) return const [];
  return ref.watch(notesTimelineProvider(pulseKeyFor(pubkeys)).future);
});

List<UserNote> _notesFromEvents(List<NostrEvent> events) {
  final notes =
      events
          .where((event) => event.kind == EventKind.note)
          .map(UserNote.fromEvent)
          .toList()
        ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
  return notes;
}

List<ContactEntry> _contactsFromTags(List<List<String>> tags) => [
  for (final tag in tags)
    if (tag.length >= 2 && tag[0] == 'p')
      ContactEntry(
        pubkey: tag[1].toLowerCase(),
        relayUrl: tag.length >= 3 && tag[2].isNotEmpty ? tag[2] : null,
        petname: tag.length >= 4 && tag[3].isNotEmpty ? tag[3] : null,
      ),
];

String? _lastETag(List<List<String>> tags) {
  for (final tag in tags.reversed) {
    if (tag.length >= 2 && tag[0] == 'e') return tag[1];
  }
  return null;
}

String pulseKeyFor(Iterable<String> values) {
  final normalized =
      values
          .map((value) => value.toLowerCase().trim())
          .where((value) => value.isNotEmpty)
          .toSet()
          .toList()
        ..sort();
  return normalized.join(',');
}

List<String> parsePulseKey(String key) {
  if (key.isEmpty) return const [];
  return key.split(',').where((value) => value.isNotEmpty).toList();
}

Future<List<NostrEvent>> _filterDeletedReactions(
  RelaySessionNotifier session,
  List<NostrEvent> reactions, {
  List<String>? deletionAuthors,
}) async {
  if (reactions.isEmpty) return const [];
  final reactionIds = reactions.map((event) => event.id).toList();
  final deletions = await session.fetchHistory(
    NostrFilters.deletionsByTargetIds(reactionIds, authors: deletionAuthors),
  );
  final deletedIds = <String>{};
  for (final deletion in deletions) {
    for (final tag in deletion.tags) {
      if (tag.length >= 2 && tag[0] == 'e') deletedIds.add(tag[1]);
    }
  }
  if (deletedIds.isEmpty) return reactions;
  return [
    for (final reaction in reactions)
      if (!deletedIds.contains(reaction.id)) reaction,
  ];
}

List<ContactEntry> contactsFromEvents(List<NostrEvent> events) {
  if (events.isEmpty) return const [];
  return _contactsFromTags(events.first.tags);
}

void preloadPulseProfiles(WidgetRef ref, List<UserNote> notes) {
  final pubkeys = <String>{};
  for (final note in notes) {
    pubkeys.add(note.pubkey);
    pubkeys.addAll(note.mentionPubkeys);
  }
  ref.read(userCacheProvider.notifier).preload(pubkeys.toList());
}
