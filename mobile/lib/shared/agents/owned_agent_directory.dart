import 'dart:async';
import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/event_signing.dart';
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
    this.runtime,
  });
  final String? model;

  /// Runtime from the verified owner-signed registration in this community.
  final String? runtime;
}

/// The owner's agents for the Agents page and for conversation pickers.
typedef OwnedAgentInventory = ({
  List<OwnedAgentDirectoryEntry> listed,
  List<OwnedAgentDirectoryEntry> recipients,
});

/// Shares one relay read across the Agents page and the conversation pickers.
/// Invalidate this inventory when an owner registration changes.
final ownedAgentInventoryProvider = FutureProvider<OwnedAgentInventory>((
  ref,
) async {
  ref.watch(relayConfigProvider);
  final owner = ref.watch(myPubkeyProvider);
  final session = ref.watch(relaySessionProvider);
  if (owner == null) {
    const empty = <OwnedAgentDirectoryEntry>[];
    return (listed: empty, recipients: empty);
  }
  // A disconnected session has not established which agents are registered.
  if (session.status != SessionStatus.connected) {
    return Completer<OwnedAgentInventory>().future;
  }
  final relay = ref.read(relaySessionProvider.notifier);
  return _loadOwnedAgentInventory(
    owner,
    (filter) => relay.queryRelay([filter]),
    includeRecipients: true,
  );
});

/// The owner's agents as desktop's Agents tab lists them. Each agent must
/// have a named registration with a well-formed `runtime` tag.
final ownedAgentDirectoryProvider =
    FutureProvider<List<OwnedAgentDirectoryEntry>>((ref) async {
      return (await ref.watch(ownedAgentInventoryProvider.future)).listed;
    });

/// Agents eligible for mentions, Add members and New DM. An owner-signed
/// named registration and a verified agent profile establish eligibility.
/// Externally hosted agents can use this path without a desktop runtime tag.
final ownedAgentRecipientsProvider =
    FutureProvider<List<OwnedAgentDirectoryEntry>>((ref) async {
      return (await ref.watch(ownedAgentInventoryProvider.future)).recipients;
    });

/// Registration pages read before failing, as `loadMeAgentInventory` bounds
/// the same inventory.
const ownedAgentDirectoryPageBudget = 50;

/// Reads the desktop-style Agents listing without fetching other profiles.
Future<List<OwnedAgentDirectoryEntry>> loadOwnedAgentDirectory(
  String owner,
  Future<List<NostrEvent>> Function(NostrFilter) query,
) async => (await _loadOwnedAgentInventory(
  owner,
  query,
  includeRecipients: false,
)).listed;

Future<OwnedAgentInventory> _loadOwnedAgentInventory(
  String owner,
  Future<List<NostrEvent>> Function(NostrFilter) query, {
  required bool includeRecipients,
}) async {
  final registrations = <String, NostrEvent>{};
  final seen = <String>{};
  for (var page = 1; ; page++) {
    if (page > ownedAgentDirectoryPageBudget) {
      throw StateError('Agent inventory exceeds the read budget.');
    }
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
  final metadata = <String, ({String? runtime, String? model})>{};
  for (final MapEntry(:key, :value) in registrations.entries) {
    final runtimeTag = _tagValue(value, 'runtime');
    final runtime = _runtimePattern.hasMatch(runtimeTag ?? '')
        ? runtimeTag
        : null;
    if (!includeRecipients && runtime == null) continue;
    final data = _namedRegistrationData(value);
    if (data == null) continue;
    final model = data['model'];
    metadata[key] = (
      runtime: runtime,
      model: model is String && model.trim().isNotEmpty ? model.trim() : null,
    );
  }
  final listed = <OwnedAgentDirectoryEntry>[];
  final recipients = <OwnedAgentDirectoryEntry>[];
  final ordered = metadata.keys.toList();
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
      if (verifiedProfileOaOwnerPubkey(event) != owner) continue;
      final profile = AgentDirectoryEntry.fromEvent(event);
      final data = metadata[event.pubkey]!;
      final signed = _hasVerifiedSignature(registrations[event.pubkey]!);
      final entry = OwnedAgentDirectoryEntry(
        pubkey: profile.pubkey,
        displayName: profile.displayName,
        avatarUrl: profile.avatarUrl,
        model: signed ? data.model : null,
        runtime: signed ? data.runtime : null,
      );
      if (data.runtime != null) listed.add(entry);
      if (includeRecipients && signed) recipients.add(entry);
    }
  }
  return (listed: listed, recipients: recipients);
}

/// An agent type key, `pluginId/typeId`, as desktop's `RUNTIME` pattern
/// (buzz-app `src/features/agents/service.ts`) accepts it.
final _runtimePattern = RegExp(r'^[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*$');

Map<String, dynamic>? _namedRegistrationData(NostrEvent event) {
  try {
    final content = jsonDecode(event.content);
    if (content is Map<String, dynamic> &&
        content['name'] is String &&
        (content['name'] as String).trim().isNotEmpty) {
      return content;
    }
  } on FormatException {
    // A malformed registration payload supplies no agent metadata.
    return null;
  }
  return null;
}

bool _hasVerifiedSignature(NostrEvent event) {
  try {
    verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    return true;
  } on nostr.EventValidationException {
    // A malformed or forged event cannot establish recipient eligibility.
    return false;
  }
}

String? _tagValue(NostrEvent event, String name) {
  for (final tag in event.tags) {
    if (tag.length >= 2 && tag.first == name) return tag[1];
  }
  return null;
}
