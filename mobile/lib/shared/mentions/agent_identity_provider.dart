import 'dart:collection';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

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
  final events = await session.fetchHistory(NostrFilters.agentProfiles());
  return [for (final event in events) AgentDirectoryEntry.fromEvent(event)];
});

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
