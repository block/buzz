import 'dart:convert';
import 'package:nostr/nostr.dart' as nostr;
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../crypto/nip_oa.dart';
import '../mentions/agent_identity_provider.dart';
import '../relay/relay.dart';

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

/// The owner's agents in the active community, as desktop's Agents tab lists
/// them: each agent's newest kind:30177 registration must name it and carry
/// the `runtime` tag of its agent type. Registrations an earlier Buzz wrote
/// have no `runtime` until the agent is imported, so they are not listed.
/// Signed profiles prove ownership.
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
      final key = _tagValue(event, 'd');
      if (key == null || !RegExp(r'^[0-9a-f]{64}$').hasMatch(key)) continue;
      final previous = registrations[key];
      if (previous == null ||
          event.createdAt > previous.createdAt ||
          (event.createdAt == previous.createdAt &&
              event.id.compareTo(previous.id) < 0)) {
        registrations[key] = event;
      }
    }
    if (events.length < 100 || fresh.isEmpty) break;
  }
  final keys = {
    for (final MapEntry(:key, :value) in registrations.entries)
      if (isListedAgentRegistration(value)) key,
  };
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

/// An agent type key, `pluginId/typeId`, as desktop's `RUNTIME` pattern
/// (buzz-app `src/features/agents/service.ts`) accepts it.
final _runtimePattern = RegExp(r'^[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*$');

/// Whether desktop lists this kind:30177 registration as one of the owner's
/// agents: it names the agent and carries a valid `runtime` tag.
bool isListedAgentRegistration(NostrEvent event) {
  if (!_runtimePattern.hasMatch(_tagValue(event, 'runtime') ?? '')) {
    return false;
  }
  try {
    final content = jsonDecode(event.content);
    return content is Map &&
        content['name'] is String &&
        (content['name'] as String).trim().isNotEmpty;
  } on FormatException {
    return false;
  }
}

String? _tagValue(NostrEvent event, String name) {
  for (final tag in event.tags) {
    if (tag.length >= 2 && tag.first == name) return tag[1];
  }
  return null;
}
