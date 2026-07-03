import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/crypto/nip_oa.dart';
import '../../../shared/relay/relay.dart';
import '../../profile/user_cache_provider.dart';
import '../channel.dart';
import '../channel_management_provider.dart';
import '../channels_provider.dart';
import 'mention_candidates.dart';
import 'mention_ranking.dart';

/// Relay agent directory from kind:10100 agent-profile events.
///
/// Watches the session so it re-fetches once connected (a fetch fired
/// before the WS is up resolves empty on timeout).
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

/// Ranked mention candidates for a channel + query. Channel members first,
/// then non-member relay agents the user can actually reach; ordering
/// matches desktop's `rankMentionCandidates`.
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
        currentPubkey: currentPubkey,
      );

      return rankMentionCandidates(candidates, args.query);
    });
