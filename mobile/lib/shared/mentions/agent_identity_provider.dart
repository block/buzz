import 'dart:collection';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/crypto/nip_oa.dart';
import '../../shared/relay/relay.dart';

/// A relay agent parsed from its kind:10100 agent-profile event.
///
/// Mirrors the fields desktop's `RelayAgent` uses for mention eligibility
/// (`agentAutocompleteEligibility.ts`): who the agent responds to and which
/// channels it sits in.
class AgentDirectoryEntry {
  final String pubkey;
  final String? displayName;
  final String? respondTo;
  final List<String> respondToAllowlist;
  final List<String> channelIds;

  const AgentDirectoryEntry({
    required this.pubkey,
    this.displayName,
    this.respondTo,
    this.respondToAllowlist = const [],
    this.channelIds = const [],
  });

  factory AgentDirectoryEntry.fromEvent(NostrEvent event) {
    final content = _tryDecodeJsonMap(event.content);
    return AgentDirectoryEntry(
      pubkey: event.pubkey.toLowerCase(),
      displayName:
          (content?['display_name'] as String?) ??
          (content?['name'] as String?),
      respondTo: content?['respond_to'] as String?,
      respondToAllowlist: [
        for (final value in (content?['respond_to_allowlist'] as List?) ?? [])
          if (value is String) value.toLowerCase(),
      ],
      channelIds: [
        for (final value in (content?['channel_ids'] as List?) ?? [])
          if (value is String) value,
      ],
    );
  }
}

Map<String, dynamic>? _tryDecodeJsonMap(String content) {
  try {
    final decoded = jsonDecode(content);
    return decoded is Map<String, dynamic> ? decoded : null;
  } catch (_) {
    return null;
  }
}

/// Relay agent directory from kind:10100 agent-profile events.
///
/// Watches the session and only fetches after the WebSocket connects.
final agentDirectoryProvider = FutureProvider<List<AgentDirectoryEntry>>((
  ref,
) async {
  final sessionState = ref.watch(relaySessionProvider);
  if (sessionState.status != SessionStatus.connected) return const [];
  final session = ref.read(relaySessionProvider.notifier);
  final results = await Future.wait([
    session.fetchHistory(NostrFilters.agentProfiles()),
    ref.watch(archivedIdentityPubkeysProvider.future),
  ]);
  return activeAgentDirectoryEntries(
    results[0] as List<NostrEvent>,
    archivedPubkeys: results[1] as Set<String>,
  );
});

/// HTTP client used to read the relay's NIP-11 signing identity.
final relayIdentityHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Relay signing pubkey advertised by NIP-11 `self`.
final relayIdentityPubkeyProvider = FutureProvider<String?>((ref) async {
  final baseUrl = ref.watch(relayConfigProvider).baseUrl;
  try {
    final response = await ref
        .read(relayIdentityHttpClientProvider)
        .get(
          Uri.parse(baseUrl),
          headers: const {'Accept': 'application/nostr+json'},
        )
        .timeout(const Duration(seconds: 5));
    if (response.statusCode < 200 || response.statusCode >= 300) return null;
    final decoded = jsonDecode(response.body);
    final relayPubkey = decoded is Map<String, dynamic>
        ? decoded['self'] as String?
        : null;
    final normalized = relayPubkey?.toLowerCase();
    return normalized != null && _isHex64(normalized) ? normalized : null;
  } catch (_) {
    return null;
  }
});

class _ArchiveIdentitySubscription extends Notifier<int> {
  void Function()? _unsubscribe;
  int _subscriptionVersion = 0;

  @override
  int build() {
    final sessionState = ref.watch(relaySessionProvider);
    final subscriptionVersion = ++_subscriptionVersion;
    _clearSubscription();
    ref.onDispose(() {
      _subscriptionVersion++;
      _clearSubscription();
    });

    if (sessionState.status != SessionStatus.connected) return 0;
    Future.microtask(() => _subscribe(subscriptionVersion));
    return 0;
  }

  Future<void> _subscribe(int subscriptionVersion) async {
    try {
      final relayPubkey = await ref.read(relayIdentityPubkeyProvider.future);
      if (relayPubkey == null || !_isCurrent(subscriptionVersion)) return;
      final session = ref.read(relaySessionProvider.notifier);
      final unsubscribe = await session.subscribe(
        NostrFilters.archivedIdentities(
          relayPubkey,
        ).copyWithSince(DateTime.now().millisecondsSinceEpoch ~/ 1000),
        (_) {
          if (_isCurrent(subscriptionVersion)) state++;
        },
      );
      if (!_isCurrent(subscriptionVersion)) {
        unsubscribe();
        return;
      }
      _unsubscribe = unsubscribe;
    } catch (error) {
      if (_isCurrent(subscriptionVersion)) {
        debugPrint('[ArchiveIdentitySubscription] failed: $error');
      }
    }
  }

  bool _isCurrent(int subscriptionVersion) =>
      subscriptionVersion == _subscriptionVersion;

  void _clearSubscription() {
    _unsubscribe?.call();
    _unsubscribe = null;
  }
}

final archiveIdentityUpdateProvider =
    NotifierProvider.autoDispose<_ArchiveIdentitySubscription, int>(
      _ArchiveIdentitySubscription.new,
    );

/// Current relay-scoped archive state, verified against NIP-11 `self`.
final archivedIdentityPubkeysProvider = FutureProvider<Set<String>>((
  ref,
) async {
  ref.watch(archiveIdentityUpdateProvider);
  final sessionState = ref.watch(relaySessionProvider);
  if (sessionState.status != SessionStatus.connected) return const {};
  final relayPubkey = await ref.watch(relayIdentityPubkeyProvider.future);
  if (relayPubkey == null) return const {};
  final session = ref.read(relaySessionProvider.notifier);
  final snapshots = await session.fetchHistory(
    NostrFilters.archivedIdentities(relayPubkey),
  );
  return archivedIdentityPubkeysFromSnapshots(
    snapshots,
    relayPubkey: relayPubkey,
    verifySignature: _hasValidNostrSignature,
  );
});

List<AgentDirectoryEntry> activeAgentDirectoryEntries(
  Iterable<NostrEvent> events, {
  required Set<String> archivedPubkeys,
}) => [
  for (final event in events)
    if (!archivedPubkeys.contains(event.pubkey.toLowerCase()))
      AgentDirectoryEntry.fromEvent(event),
];

Set<String> archivedIdentityPubkeysFromSnapshots(
  Iterable<NostrEvent> snapshots, {
  required String relayPubkey,
  required bool Function(NostrEvent event) verifySignature,
}) {
  final normalizedRelayPubkey = relayPubkey.toLowerCase();
  final valid =
      snapshots
          .where(
            (event) =>
                event.kind == 13535 &&
                event.pubkey.toLowerCase() == normalizedRelayPubkey &&
                _hasExactlyOneNip70Tag(event.tags) &&
                verifySignature(event),
          )
          .toList()
        ..sort((a, b) {
          final byTimestamp = b.createdAt.compareTo(a.createdAt);
          return byTimestamp != 0 ? byTimestamp : a.id.compareTo(b.id);
        });
  if (valid.isEmpty) return const {};
  return {
    for (final tag in valid.first.tags)
      if (tag.length >= 2 && tag[0] == 'p' && _isHex64(tag[1]))
        tag[1].toLowerCase(),
  };
}

bool _hasExactlyOneNip70Tag(List<List<String>> tags) {
  var count = 0;
  for (final tag in tags) {
    if (tag.isEmpty || tag.first != '-') continue;
    if (tag.length != 1) return false;
    count++;
  }
  return count == 1;
}

bool _hasValidNostrSignature(NostrEvent event) {
  try {
    nostr.Event.fromJson(jsonEncode(event.toJson()));
    return true;
  } catch (_) {
    return false;
  }
}

bool _isHex64(String value) =>
    value.length == 64 && RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(value);

/// Verified NIP-OA owner pubkey per agent pubkey, from the agents' kind:0
/// profiles. An entry exists only when the `auth` tag verifies — mirrors
/// desktop's `profile_valid_oa_owner_pubkey`.
final agentOwnersProvider = FutureProvider<Map<String, String>>((ref) async {
  final agents = await ref.watch(agentDirectoryProvider.future);
  if (agents.isEmpty) return const {};
  final session = ref.read(relaySessionProvider.notifier);
  final events = await session.fetchHistory(
    NostrFilters.profilesBatch([for (final agent in agents) agent.pubkey]),
  );
  final owners = <String, String>{};
  for (final event in events) {
    final owner = verifiedOaOwnerPubkey(event.tags, event.pubkey);
    if (owner != null) owners[event.pubkey.toLowerCase()] = owner;
  }
  return owners;
});

/// Pubkeys currently known to represent agents across the active relay.
///
/// Message surfaces that do not own channel membership can use this shared
/// identity source; channel features add their bot roles separately.
final knownAgentPubkeysProvider = Provider<Set<String>>((ref) {
  final relayAgents =
      ref.watch(agentDirectoryProvider).asData?.value ??
      const <AgentDirectoryEntry>[];
  final owners = ref.watch(agentOwnersProvider).asData?.value ?? const {};
  return _AgentPubkeySet({
    for (final agent in relayAgents) agent.pubkey.toLowerCase(),
    ...owners.keys.map((pubkey) => pubkey.toLowerCase()),
  });
});

/// Directory display names keyed by agent pubkey for mention presentation.
final agentDirectoryDisplayNamesProvider = Provider<Map<String, String>>((ref) {
  final agents =
      ref.watch(agentDirectoryProvider).asData?.value ??
      const <AgentDirectoryEntry>[];
  return Map.unmodifiable({
    for (final agent in agents)
      if (agent.displayName?.trim().isNotEmpty == true)
        agent.pubkey.toLowerCase(): agent.displayName!.trim(),
  });
});

/// Adds channel bot roles to relay-wide agent identities.
Set<String> agentPubkeysWithChannelBots({
  required Set<String> knownAgentPubkeys,
  required Iterable<String> channelBotPubkeys,
}) => _AgentPubkeySet({
  ...knownAgentPubkeys,
  ...channelBotPubkeys.map((pubkey) => pubkey.toLowerCase()),
});

/// Adds agent identities derived from locally cached verified profiles.
Set<String> agentPubkeysWithProfileOwners({
  required Set<String> knownAgentPubkeys,
  required Iterable<String> profileOwnedAgentPubkeys,
}) => _AgentPubkeySet({
  ...knownAgentPubkeys,
  ...profileOwnedAgentPubkeys.map((pubkey) => pubkey.toLowerCase()),
});

/// Preserves profile labels while filling missing agent mentions from the
/// relay's agent directory.
Map<String, String> mentionNamesWithDirectoryLabels({
  required Iterable<String> mentionPubkeys,
  required Map<String, String> profileMentionNames,
  required Map<String, String> directoryDisplayNames,
  required Set<String> agentMentionPubkeys,
}) {
  final names = Map<String, String>.from(profileMentionNames);
  for (final pubkey in mentionPubkeys) {
    final normalizedPubkey = pubkey.toLowerCase();
    if (names[normalizedPubkey]?.trim().isEmpty == true) {
      names.remove(normalizedPubkey);
    }
    final directoryName = directoryDisplayNames[normalizedPubkey];
    if (!names.containsKey(normalizedPubkey) && directoryName != null) {
      names[normalizedPubkey] = directoryName;
    }
    if (!names.containsKey(normalizedPubkey) &&
        agentMentionPubkeys.contains(normalizedPubkey)) {
      names[normalizedPubkey] = _agentFallbackLabel(normalizedPubkey);
    }
  }
  return names;
}

String _agentFallbackLabel(String pubkey) =>
    pubkey.length >= 8 ? pubkey.substring(0, 8) : pubkey;

/// Keeps the role feed alive for consumers that render mentions outside the
/// channel timeline, such as search results. A membership change refreshes the
/// shared bot-role lookup below, regardless of which surface owns the channel.
class _ChannelBotRoleSubscription extends Notifier<int> {
  final String channelId;
  void Function()? _unsubscribe;
  int _subscriptionVersion = 0;

  _ChannelBotRoleSubscription(this.channelId);

  @override
  int build() {
    final sessionState = ref.watch(relaySessionProvider);
    final subscriptionVersion = ++_subscriptionVersion;
    _clearSubscription();
    ref.onDispose(() {
      _subscriptionVersion++;
      _clearSubscription();
    });

    if (sessionState.status != SessionStatus.connected) return 0;
    Future.microtask(() => _subscribe(channelId, subscriptionVersion));
    return 0;
  }

  Future<void> _subscribe(String channelId, int subscriptionVersion) async {
    final session = ref.read(relaySessionProvider.notifier);
    try {
      final unsubscribe = await session.subscribe(
        NostrFilter(
          kinds: const [39002],
          tags: {
            '#h': [channelId],
          },
        ).copyWithSince(DateTime.now().millisecondsSinceEpoch ~/ 1000),
        (_) {
          if (_isCurrent(subscriptionVersion)) {
            state++;
          }
        },
      );
      if (!_isCurrent(subscriptionVersion)) {
        unsubscribe();
        return;
      }
      _unsubscribe = unsubscribe;
    } catch (error) {
      if (_isCurrent(subscriptionVersion)) {
        debugPrint(
          '[ChannelBotRoleSubscription] failed for $channelId: $error',
        );
      }
    }
  }

  bool _isCurrent(int subscriptionVersion) =>
      subscriptionVersion == _subscriptionVersion;

  void _clearSubscription() {
    _unsubscribe?.call();
    _unsubscribe = null;
  }
}

/// Monotonically increments when the channel's kind:39002 membership snapshot
/// changes. Channel-member and agent-role views share this source so remote
/// membership updates refresh both snapshots together.
final channelMembershipUpdateProvider = NotifierProvider.autoDispose
    .family<_ChannelBotRoleSubscription, int, String>(
      _ChannelBotRoleSubscription.new,
    );

/// Bot pubkeys currently assigned a channel bot role.
final channelBotPubkeysProvider = FutureProvider.autoDispose
    .family<Set<String>, String>((ref, channelId) async {
      ref.watch(channelMembershipUpdateProvider(channelId));
      final sessionState = ref.watch(relaySessionProvider);
      if (sessionState.status != SessionStatus.connected) return const {};
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilters.channelMembers(channelId),
      );
      if (events.isEmpty) return const {};
      return _AgentPubkeySet({
        for (final member in membersFromEvent(events.first))
          if (member.role == 'bot') member.pubkey.toLowerCase(),
      });
    });

/// Pubkeys currently known to represent agents in a channel.
final agentMentionPubkeysProvider = Provider.autoDispose
    .family<Set<String>, String>((ref, channelId) {
      final channelBotPubkeys =
          ref.watch(channelBotPubkeysProvider(channelId)).asData?.value ??
          const <String>{};
      return agentPubkeysWithChannelBots(
        knownAgentPubkeys: ref.watch(knownAgentPubkeysProvider),
        channelBotPubkeys: channelBotPubkeys,
      );
    });

class _AgentPubkeySet extends UnmodifiableSetView<String> {
  _AgentPubkeySet(Iterable<String> pubkeys) : super(Set.unmodifiable(pubkeys));

  @override
  bool operator ==(Object other) =>
      other is Set && length == other.length && every(other.contains);

  @override
  int get hashCode => Object.hashAllUnordered(this);
}
