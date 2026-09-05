import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/crypto/nip_oa.dart';
import '../../shared/relay/relay.dart';

final _hexPubkey = RegExp(r'^[0-9a-f]{64}$', caseSensitive: false);

/// HTTP transport used to discover the active relay's NIP-11 `self` key.
/// Tests override this provider; production owns and closes one client.
final archivedIdentityHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// A verified relay-scoped NIP-IA snapshot.
@immutable
class ArchivedIdentitySnapshot {
  final String eventId;
  final int createdAt;
  final Set<String> pubkeys;

  const ArchivedIdentitySnapshot({
    required this.eventId,
    required this.createdAt,
    required this.pubkeys,
  });
}

/// Reads the stable relay identity advertised by NIP-11.
///
/// Invalid or unavailable relay metadata fails open: callers receive `null`
/// and do not hide any identity.
Future<String?> fetchRelayIdentityPubkey(
  http.Client client,
  String relayUrl,
) async {
  try {
    final uri = Uri.parse(relayUrl.trim());
    final response = await client
        .get(uri, headers: const {'Accept': 'application/nostr+json'})
        .timeout(const Duration(seconds: 5));
    if (response.statusCode < 200 || response.statusCode >= 300) return null;
    final document = jsonDecode(response.body);
    if (document is! Map<String, dynamic>) return null;
    final relayPubkey = document['self'];
    if (relayPubkey is! String || !_hexPubkey.hasMatch(relayPubkey)) {
      return null;
    }
    return relayPubkey.toLowerCase();
  } catch (_) {
    return null;
  }
}

/// Verifies and parses one NIP-IA `kind:13535` snapshot.
///
/// Trust requires all three anchors: the NIP-11 relay author, a valid NIP-01
/// id/signature, and exactly one NIP-70 `["-"]` marker. Malformed `p` tags
/// are ignored rather than allowed to poison the complete snapshot.
ArchivedIdentitySnapshot? parseArchivedIdentitySnapshot(
  NostrEvent event,
  String relayPubkey,
) {
  final normalizedRelayPubkey = relayPubkey.toLowerCase();
  if (event.kind != EventKind.archivedIdentities ||
      event.pubkey.toLowerCase() != normalizedRelayPubkey ||
      !_hexPubkey.hasMatch(normalizedRelayPubkey)) {
    return null;
  }
  final nip70Markers = event.tags
      .where((tag) => tag.isNotEmpty && tag.first == '-')
      .toList();
  if (nip70Markers.length != 1 || nip70Markers.single.length != 1) {
    return null;
  }
  try {
    final verified = nostr.Event.fromJson(jsonEncode(event.toJson()));
    if (verified.id != event.id) return null;
  } catch (_) {
    return null;
  }

  return ArchivedIdentitySnapshot(
    eventId: event.id,
    createdAt: event.createdAt,
    pubkeys: Set.unmodifiable({
      for (final tag in event.tags)
        if (tag.length >= 2 && tag.first == 'p' && _hexPubkey.hasMatch(tag[1]))
          tag[1].toLowerCase(),
    }),
  );
}

/// NIP-01 replacement ordering: newest timestamp wins; equal timestamps keep
/// the lexicographically lowest event id.
ArchivedIdentitySnapshot? latestArchivedIdentitySnapshot(
  Iterable<NostrEvent> events,
  String relayPubkey,
) {
  ArchivedIdentitySnapshot? latest;
  for (final event in events) {
    final candidate = parseArchivedIdentitySnapshot(event, relayPubkey);
    if (candidate == null) continue;
    if (latest == null ||
        candidate.createdAt > latest.createdAt ||
        (candidate.createdAt == latest.createdAt &&
            candidate.eventId.compareTo(latest.eventId) < 0)) {
      latest = candidate;
    }
  }
  return latest;
}

class ArchivedIdentityPubkeysNotifier extends AsyncNotifier<Set<String>> {
  void Function()? _unsubscribe;
  ArchivedIdentitySnapshot? _latest;
  int _generation = 0;

  @override
  Future<Set<String>> build() async {
    final relayConfig = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    final generation = ++_generation;
    _clearSubscription();
    ref.onDispose(() {
      _generation++;
      _clearSubscription();
    });

    if (sessionState.status != SessionStatus.connected) return const {};
    final relayPubkey = await fetchRelayIdentityPubkey(
      ref.read(archivedIdentityHttpClientProvider),
      relayConfig.baseUrl,
    );
    if (relayPubkey == null || generation != _generation) return const {};

    final session = ref.read(relaySessionProvider.notifier);
    try {
      final events = await session.fetchHistory(
        NostrFilters.archivedIdentities(relayPubkey),
      );
      if (generation != _generation) return const {};
      _latest = latestArchivedIdentitySnapshot(events, relayPubkey);
      unawaited(_subscribe(session, relayPubkey, generation));
      return _latest?.pubkeys ?? const {};
    } catch (error) {
      debugPrint('[ArchivedIdentities] snapshot fetch failed: $error');
      return const {};
    }
  }

  Future<void> _subscribe(
    RelaySessionNotifier session,
    String relayPubkey,
    int generation,
  ) async {
    // Overlap the history/live handoff so a snapshot published in the gap is
    // replayed instead of missed. Replacement ordering deduplicates it.
    final since =
        (_latest?.createdAt ?? DateTime.now().millisecondsSinceEpoch ~/ 1000) -
        5;
    try {
      final unsubscribe = await session.subscribe(
        NostrFilters.archivedIdentities(relayPubkey).copyWithSince(since),
        (event) => _acceptLiveSnapshot(event, relayPubkey, generation),
      );
      if (generation != _generation) {
        unsubscribe();
        return;
      }
      _unsubscribe = unsubscribe;
    } catch (error) {
      if (generation == _generation) {
        debugPrint('[ArchivedIdentities] live subscription failed: $error');
      }
    }
  }

  void _acceptLiveSnapshot(
    NostrEvent event,
    String relayPubkey,
    int generation,
  ) {
    if (generation != _generation) return;
    final candidate = parseArchivedIdentitySnapshot(event, relayPubkey);
    if (candidate == null) return;
    final latest = _latest;
    if (latest != null &&
        (candidate.createdAt < latest.createdAt ||
            (candidate.createdAt == latest.createdAt &&
                candidate.eventId.compareTo(latest.eventId) >= 0))) {
      return;
    }
    _latest = candidate;
    state = AsyncData(candidate.pubkeys);
  }

  void _clearSubscription() {
    _unsubscribe?.call();
    _unsubscribe = null;
    _latest = null;
  }
}

/// Relay-scoped identities hidden from forward-looking discovery surfaces.
/// Fail-open while disconnected, loading, or when relay proof is invalid.
final archivedIdentityPubkeysProvider =
    AsyncNotifierProvider<ArchivedIdentityPubkeysNotifier, Set<String>>(
      ArchivedIdentityPubkeysNotifier.new,
    );

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

/// Readiness and refresh state for a channel's live membership subscription.
class ChannelMembershipUpdateState {
  final int version;
  final bool isReady;
  final Object? error;

  const ChannelMembershipUpdateState({
    this.version = 0,
    this.isReady = false,
    this.error,
  });
}

/// Keeps the role feed alive for consumers that render mentions outside the
/// channel timeline, such as search results. A membership change refreshes the
/// shared bot-role lookup below, regardless of which surface owns the channel.
class _ChannelBotRoleSubscription
    extends Notifier<ChannelMembershipUpdateState> {
  final String channelId;
  void Function()? _unsubscribe;
  int _subscriptionVersion = 0;

  _ChannelBotRoleSubscription(this.channelId);

  @override
  ChannelMembershipUpdateState build() {
    final sessionState = ref.watch(relaySessionProvider);
    final subscriptionVersion = ++_subscriptionVersion;
    _clearSubscription();
    ref.onDispose(() {
      _subscriptionVersion++;
      _clearSubscription();
    });

    if (sessionState.status != SessionStatus.connected) {
      return const ChannelMembershipUpdateState();
    }
    Future.microtask(() => _subscribe(channelId, subscriptionVersion));
    return const ChannelMembershipUpdateState();
  }

  Future<void> _subscribe(String channelId, int subscriptionVersion) async {
    final session = ref.read(relaySessionProvider.notifier);
    var subscriptionStatus = RelaySubscriptionStatus.retrying;
    try {
      final unsubscribe = await session.subscribeWithStatus(
        NostrFilter(
          kinds: const [39002],
          tags: {
            '#d': [channelId],
          },
        ).copyWithSince(DateTime.now().millisecondsSinceEpoch ~/ 1000),
        (_) {
          if (_isCurrent(subscriptionVersion)) {
            state = ChannelMembershipUpdateState(
              version: state.version + 1,
              isReady: state.isReady,
            );
          }
        },
        onClosed: (message) {
          if (_isCurrent(subscriptionVersion)) {
            state = ChannelMembershipUpdateState(
              version: state.version,
              error: Exception(message),
            );
          }
        },
        onStatusChanged: (status) {
          subscriptionStatus = status;
          if (!_isCurrent(subscriptionVersion)) return;
          state = ChannelMembershipUpdateState(
            version: state.version,
            isReady: status == RelaySubscriptionStatus.ready,
          );
        },
      );
      if (!_isCurrent(subscriptionVersion)) {
        unsubscribe();
        return;
      }
      _unsubscribe = unsubscribe;
      state = ChannelMembershipUpdateState(
        version: state.version,
        isReady: subscriptionStatus == RelaySubscriptionStatus.ready,
      );
    } catch (error) {
      if (_isCurrent(subscriptionVersion)) {
        state = ChannelMembershipUpdateState(
          version: state.version,
          error: error,
        );
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
    .family<_ChannelBotRoleSubscription, ChannelMembershipUpdateState, String>(
      _ChannelBotRoleSubscription.new,
    );

/// Bot pubkeys currently assigned a channel bot role.
final channelBotPubkeysProvider = FutureProvider.autoDispose
    .family<Set<String>, String>((ref, channelId) async {
      ref.watch(
        channelMembershipUpdateProvider(
          channelId,
        ).select((update) => update.version),
      );
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
