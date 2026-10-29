import 'dart:convert';
import 'package:nostr/nostr.dart' as nostr;
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/crypto/nip_oa.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/relay/relay.dart';

/// Public model information accompanying a verified owned agent profile.
class OwnedAgentDirectoryEntry extends AgentDirectoryEntry {
  const OwnedAgentDirectoryEntry({
    required super.pubkey,
    super.displayName,
    super.avatarUrl,
    this.model,
  });
  final String? model;
}

/// Discovers owned identities even when their runtime has no kind:10100 entry.
/// Owner registrations nominate candidates; signed profiles prove ownership.
final ownedAgentDirectoryProvider =
    FutureProvider<List<OwnedAgentDirectoryEntry>>((ref) async {
      ref.watch(relayConfigProvider);
      final owner = ref.watch(myPubkeyProvider);
      final session = ref.watch(relaySessionProvider);
      if (owner == null || session.status != SessionStatus.connected) {
        return const [];
      }
      final relay = ref.read(relaySessionProvider.notifier);
      return loadOwnedAgentDirectory(
        owner,
        (filter) => relay.queryRelay([filter]),
      );
    });

/// Uses the active community's authenticated query boundary and paginates it.
Future<List<OwnedAgentDirectoryEntry>> loadOwnedAgentDirectory(
  String owner,
  Future<List<NostrEvent>> Function(NostrFilter) query,
) async {
  final keys = <String>{};
  final registrations = <String, NostrEvent>{};
  final seen = <String>{};
  for (var page = 1; ; page++) {
    final events = await query(
      NostrFilter(
        kinds: const [30177],
        authors: [owner],
        limit: 100,
        extensions: {'page': page},
      ),
    );
    final fresh = events.where((event) => seen.add(event.id)).toList();
    for (final event in fresh) {
      if (event.kind != 30177 || event.pubkey != owner) continue;
      for (final tag in event.tags) {
        if (tag.length >= 2 &&
            tag.first == 'd' &&
            RegExp(r'^[0-9a-f]{64}$').hasMatch(tag[1])) {
          keys.add(tag[1]);
          final previous = registrations[tag[1]];
          if (previous == null ||
              event.createdAt > previous.createdAt ||
              (event.createdAt == previous.createdAt &&
                  event.id.compareTo(previous.id) < 0)) {
            registrations[tag[1]] = event;
          }
        }
      }
    }
    if (events.length < 100 || fresh.isEmpty) break;
  }
  final results = <OwnedAgentDirectoryEntry>[];
  final ordered = keys.toList();
  for (var offset = 0; offset < ordered.length; offset += 50) {
    final batch = ordered.skip(offset).take(50).toList();
    final events = await query(NostrFilters.profilesBatch(batch));
    final latest = <String, NostrEvent>{};
    for (final event in events) {
      if (event.kind != 0 || !batch.contains(event.pubkey)) continue;
      final previous = latest[event.pubkey];
      if (previous == null ||
          event.createdAt > previous.createdAt ||
          (event.createdAt == previous.createdAt &&
              event.id.compareTo(previous.id) < 0)) {
        latest[event.pubkey] = event;
      }
    }
    for (final event in latest.values) {
      if (verifiedProfileOaOwnerPubkey(event) == owner) {
        final profile = AgentDirectoryEntry.fromEvent(event);
        String? model;
        try {
          final registration = registrations[event.pubkey]!;
          nostr.Event.fromJson(jsonEncode(registration.toJson()));
          final data = jsonDecode(registration.content);
          if (data is Map && data['model'] is String) {
            final value = (data['model'] as String).trim();
            if (value.isNotEmpty) model = value;
          }
        } catch (_) {
          /* Older registrations may have no metadata. */
        }
        results.add(
          OwnedAgentDirectoryEntry(
            pubkey: profile.pubkey,
            displayName: profile.displayName,
            avatarUrl: profile.avatarUrl,
            model: model,
          ),
        );
      }
    }
  }
  return results;
}
