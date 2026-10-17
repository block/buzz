import 'dart:convert';

import 'package:nostr/nostr.dart' as nostr;
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../crypto/nip_oa.dart';
import '../mentions/agent_identity_provider.dart';
import '../relay/relay.dart';
import 'me_sync.dart';

/// Owned assistants from verified profiles, including agents without kind 10100.
/// Agents inherit their owner's community access and need not be listed in the
/// community's human membership snapshot.
final meAssistantsProvider = FutureProvider<List<AgentDirectoryEntry>>((
  ref,
) async {
  final sync = ref.watch(meSyncProvider);
  if (sync.connectionState != null) {
    ref.watch(relaySessionProvider.select((state) => state.status));
  }
  final session = ref.watch(relaySessionProvider.notifier);
  return _loadAssistants(sync, session);
}, retry: (_, _) => null);

/// Freshness-controlled ownership check for one personal-chat recipient.
final meAssistantOwnershipProvider = FutureProvider.family<bool, String>((
  ref,
  pubkey,
) {
  final sync = ref.watch(meSyncProvider);
  if (sync.connectionState != null) {
    ref.watch(relaySessionProvider.select((state) => state.status));
  }
  return isOwnedMeAssistant(sync, pubkey);
});

/// Revalidates only the chosen assistant before creating a chat.
Future<bool> isOwnedMeAssistant(MeSync sync, String pubkey) async =>
    (await _loadAssistants(
      sync,
      sync.session,
      pubkey: pubkey,
    )).any((agent) => agent.pubkey == pubkey);

Future<List<AgentDirectoryEntry>> _loadAssistants(
  MeSync sync,
  RelaySessionNotifier session, {
  String? pubkey,
}) async {
  final profiles = <String, NostrEvent>{};
  final seen = <String>{};
  const pageSize = 100;
  // The current relay only indexes single-letter NIP-01 tag filters. It
  // silently drops #auth, so owner-scoped discovery needs relay support.
  // Until then fail visibly at this bound rather than drain the directory.
  const maxPages = 10;
  for (var page = 1; page <= maxPages; page++) {
    sync.check();
    final events = await session.queryRelay([
      NostrFilter(
        kinds: const [0],
        authors: pubkey == null ? null : [pubkey],
        limit: pageSize,
        extensions: {'page': page},
      ),
    ]);
    sync.check();
    if (events.length > pageSize) {
      throw StateError('Agent directory exceeded the requested page size.');
    }
    var added = false;
    for (final event in events) {
      if (event.kind != 0 || (pubkey != null && event.pubkey != pubkey)) {
        continue;
      }
      if (seen.contains(event.id)) continue;
      try {
        nostr.Event.fromJson(jsonEncode(event.toJson()));
      } catch (_) {
        continue;
      }
      if (!seen.add(event.id)) continue;
      added = true;
      final previous = profiles[event.pubkey];
      if (previous == null ||
          event.createdAt > previous.createdAt ||
          (event.createdAt == previous.createdAt &&
              event.id.compareTo(previous.id) < 0)) {
        profiles[event.pubkey] = event;
      }
    }
    if (events.length < pageSize) break;
    if (!added || page == maxPages) {
      throw StateError('Could not finish loading assistants. Try again.');
    }
  }
  final agents = <AgentDirectoryEntry>[];
  for (final event in profiles.values) {
    try {
      if (verifiedProfileOaOwnerPubkey(event) == sync.pubkey) {
        agents.add(AgentDirectoryEntry.fromEvent(event));
      }
    } catch (_) {
      // A malformed profile must not hide unrelated verified agents.
      continue;
    }
  }
  return agents;
}
