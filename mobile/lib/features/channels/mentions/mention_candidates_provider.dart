import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/crypto/nip_oa.dart';
import '../../../shared/relay/relay.dart';
import '../../profile/user_cache_provider.dart';
import '../../profile/user_profile.dart';
import '../channel.dart';
import '../channel_management_provider.dart';
import '../channels_provider.dart';
import 'mention_candidates.dart';
import 'mention_ranking.dart';

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

/// Debounce before a mention query hits the relay search endpoint.
const _mentionSearchDebounce = Duration(milliseconds: 250);

/// Global user search for mention autocomplete — kind:0 prefix search via
/// the relay HTTP bridge. Mirrors desktop's `useInfiniteUserSearchQuery`
/// feeding `useMentions` (source 4: people and agents outside the channel).
///
/// Debounced: the provider waits [_mentionSearchDebounce] before querying;
/// keystrokes dispose the stale family member so its request never fires.
final mentionUserSearchProvider = FutureProvider.autoDispose
    .family<List<UserProfile>, String>((ref, query) async {
      final trimmed = query.trim();
      if (trimmed.isEmpty) return const [];

      var disposed = false;
      ref.onDispose(() => disposed = true);
      await Future<void>.delayed(_mentionSearchDebounce);
      if (disposed) return const [];

      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.queryRelay([
        NostrFilters.searchUsers(trimmed),
      ]);

      // Keep only the latest kind:0 event per pubkey (the bridge does not
      // honor the `kinds` filter under search, and may return several
      // profile revisions — mirrors desktop's `list_user_search_results`).
      final latestByPubkey = <String, NostrEvent>{};
      for (final event in events) {
        if (event.kind != 0) continue;
        final pk = event.pubkey.toLowerCase();
        final current = latestByPubkey[pk];
        if (current == null || event.createdAt > current.createdAt) {
          latestByPubkey[pk] = event;
        }
      }

      return [
        for (final event in latestByPubkey.values) _profileFromEvent(event),
      ];
    });

UserProfile _profileFromEvent(NostrEvent event) {
  final data = ProfileData.fromEvent(event);
  return UserProfile(
    pubkey: event.pubkey.toLowerCase(),
    displayName: data.displayName,
    avatarUrl: data.avatarUrl,
    about: data.about,
    nip05Handle: data.nip05,
    ownerPubkey: verifiedOaOwnerPubkey(event.tags, event.pubkey),
  );
}

/// Ranked mention candidates for a channel + query. Channel members first,
/// then non-member relay agents the user can actually reach, then global
/// search results; ordering matches desktop's `rankMentionCandidates`.
final mentionCandidatesProvider = Provider.family
    .autoDispose<List<MentionCandidate>, ({String channelId, String query})>((
      ref,
      args,
    ) {
      final members =
          ref.watch(channelMembersProvider(args.channelId)).asData?.value ??
          const <ChannelMember>[];
      final relayAgents =
          ref.watch(agentDirectoryProvider).asData?.value ??
          const <AgentDirectoryEntry>[];
      final owners = ref.watch(agentOwnersProvider).asData?.value ?? const {};
      final channels =
          ref.watch(channelsProvider).asData?.value ?? const <Channel>[];
      final userCache = ref.watch(userCacheProvider);
      final currentPubkey = ref.watch(currentPubkeyProvider);
      final searchResults =
          ref.watch(mentionUserSearchProvider(args.query)).asData?.value ??
          const <UserProfile>[];

      final sharedChannelIds = {
        for (final channel in channels)
          if (channel.isMember && !channel.isArchived) channel.id,
      };

      final candidates = buildMentionCandidates(
        members: members,
        relayAgents: relayAgents,
        sharedChannelIds: sharedChannelIds,
        userCache: userCache,
        ownerByAgentPubkey: owners,
        searchResults: searchResults,
        currentPubkey: currentPubkey,
      );

      return rankMentionCandidates(
        candidates,
        args.query,
        currentPubkey: currentPubkey,
      );
    });
