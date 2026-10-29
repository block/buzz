import 'dart:convert';
import 'dart:isolate';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../crypto/nip_oa.dart';
import '../mentions/agent_identity_provider.dart';
import '../relay/relay.dart';
import 'me_agent_inventory.dart';
import 'me_archived_agents.dart';
import 'me_sync.dart';

/// The owner's current inventory, with verified ownership and desktop's
/// known-archived exclusion. Never scan the community's historical profiles.
final meAssistantsProvider = FutureProvider<List<AgentDirectoryEntry>>((
  ref,
) async {
  final sync = ref.watch(meSyncProvider);
  if (sync.connectionState != null) {
    ref.watch(relaySessionProvider.select((state) => state.status));
  }
  // Refresh archive evidence with the inventory, including explicit retries.
  // Ordinary sheet openings reuse this provider's completed directory.
  final archivedRequest = ref.refresh(meArchivedAgentsProvider.future);
  final inventory = await loadMeAgentInventory(sync);
  sync.check();
  final profiles = await _loadProfiles(sync, inventory.keys.toList());
  final archived = await archivedRequest;
  sync.check();
  final choices = [
    for (final profile in profiles)
      if (!archived.contains(profile.pubkey))
        AgentDirectoryEntry(
          pubkey: profile.pubkey,
          displayName: inventory[profile.pubkey],
          avatarUrl: profile.avatarUrl,
          respondTo: profile.respondTo,
          respondToAllowlist: profile.respondToAllowlist,
          channelIds: profile.channelIds,
        ),
  ];
  choices.sort(
    (a, b) => (a.displayName ?? '').toLowerCase().compareTo(
      (b.displayName ?? '').toLowerCase(),
    ),
  );
  return choices;
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

/// Revalidate the selected recipient, including existing chats whose agent may
/// no longer be offered in the new-chat inventory. Discovery is not admission.
Future<bool> isOwnedMeAssistant(MeSync sync, String pubkey) async =>
    (await _loadProfiles(sync, [
      pubkey,
    ])).any((agent) => agent.pubkey == pubkey);

Future<List<AgentDirectoryEntry>> _loadProfiles(
  MeSync sync,
  List<String> pubkeys,
) async {
  final profiles = <NostrEvent>[];
  const batchSize = 100;
  for (var start = 0; start < pubkeys.length; start += batchSize) {
    sync.check();
    final batch = pubkeys.skip(start).take(batchSize).toList();
    final events = await sync.session.queryRelay([
      NostrFilter(kinds: const [0], authors: batch, limit: batchSize),
    ]);
    sync.check();
    if (events.length > batchSize) {
      throw StateError('Agent profiles exceeded the requested batch size.');
    }
    profiles.addAll(
      events.where((e) => e.kind == 0 && batch.contains(e.pubkey)),
    );
  }
  final owner = sync.pubkey;
  final result = await _parseProfiles(profiles, owner);
  sync.check();
  return result;
}

Future<List<AgentDirectoryEntry>> _parseProfiles(
  List<NostrEvent> events,
  String owner,
) => Isolate.run(() => _verifiedProfiles(events, owner));

List<AgentDirectoryEntry> _verifiedProfiles(
  List<NostrEvent> events,
  String owner,
) {
  final latest = <String, NostrEvent>{};
  for (final event in events) {
    try {
      verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    } catch (_) {
      continue;
    }
    final previous = latest[event.pubkey];
    if (previous == null || meEventIsNewer(event, previous)) {
      latest[event.pubkey] = event;
    }
  }
  return [
    for (final event in latest.values)
      if (verifiedProfileOaOwnerPubkey(event) == owner)
        AgentDirectoryEntry.fromEvent(event),
  ];
}
