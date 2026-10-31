import '../../../shared/agents/owned_agent_directory.dart';
import '../../../shared/mentions/agent_identity_provider.dart';
import '../../../shared/profile/user_profile.dart';
import '../../../shared/utils/string_utils.dart';
import '../channel_management_provider.dart';
import 'mention_ranking.dart';

/// Whether a non-member relay agent should be mentionable by the current
/// user. Mirrors desktop's `relayAgentIsSharedWithUser`:
/// - allowlist mode: user must be on the allowlist
/// - anyone mode: agent must share at least one channel with the user
bool agentIsSharedWithUser(
  AgentDirectoryEntry agent,
  Set<String> sharedChannelIds,
  String? currentPubkey,
) {
  if (agent.respondTo == 'allowlist' && currentPubkey != null) {
    return agent.respondToAllowlist.contains(currentPubkey.toLowerCase());
  }
  return agent.respondTo == 'anyone' &&
      agent.channelIds.any(sharedChannelIds.contains);
}

/// Format the "managed by …" label. Mirrors desktop's `formatOwnerLabel`.
String? formatOwnerLabel(
  String? ownerPubkey,
  String? currentPubkey,
  Map<String, UserProfile> userCache,
) {
  if (ownerPubkey == null) return null;
  final owner = ownerPubkey.toLowerCase();
  if (currentPubkey != null && owner == currentPubkey.toLowerCase()) {
    return 'you';
  }
  final profile = userCache[owner];
  final name = profile?.displayName?.trim();
  if (name != null && name.isNotEmpty) return name;
  final handle = profile?.nip05Handle?.trim();
  if (handle != null && handle.isNotEmpty) return handle;
  return shortPubkey(ownerPubkey);
}

/// Assemble the full mention candidate list: channel members first-class,
/// then the viewer's own agents where they can be invited, then eligible
/// non-member relay agents, then global user-search results.
///
/// [ownedAgents] are the viewer's agents as desktop's Agents tab lists them
/// (`ownedAgentDirectoryProvider`). Outside the channel, an agent the viewer
/// owns is offered only when it is one of these: one only an earlier Buzz
/// runs cannot answer until desktop imports it, which mobile cannot do.
/// [offerOwnedAgents] offers them unprompted, as desktop does in streams and
/// forums.
List<MentionCandidate> buildMentionCandidates({
  required List<ChannelMember> members,
  required List<AgentDirectoryEntry> relayAgents,
  required Set<String> sharedChannelIds,
  required Map<String, UserProfile> userCache,
  required Map<String, String> ownerByAgentPubkey,
  List<UserProfile> searchResults = const [],
  String? currentPubkey,
  List<OwnedAgentDirectoryEntry> ownedAgents = const [],
  bool offerOwnedAgents = false,
}) {
  final candidates = <MentionCandidate>[];
  final seen = <String>{};
  final currentLower = currentPubkey?.toLowerCase();
  final listedOwned = {for (final agent in ownedAgents) agent.pubkey};
  bool unlistedOwnAgent(String pubkey, String? ownerPubkey) =>
      currentLower != null &&
      ownerPubkey?.toLowerCase() == currentLower &&
      !listedOwned.contains(pubkey);

  for (final member in members) {
    final pk = member.pubkey.toLowerCase();
    if (!seen.add(pk)) continue;
    final profile = userCache[pk];
    final ownerPubkey = ownerByAgentPubkey[pk] ?? profile?.ownerPubkey;
    final isAgent = member.isBot || ownerPubkey != null;
    candidates.add(
      MentionCandidate(
        pubkey: pk,
        displayName: profile?.displayName?.trim().isNotEmpty == true
            ? profile!.displayName!.trim()
            : member.displayName,
        secondaryLabel: profile?.nip05Handle,
        avatarUrl: profile?.avatarUrl,
        isAgent: isAgent,
        isMember: true,
        role: member.role,
        ownerPubkey: ownerPubkey,
      ),
    );
  }

  if (offerOwnedAgents) {
    for (final agent in ownedAgents) {
      final pk = agent.pubkey;
      if (!seen.add(pk)) continue;
      final profile = userCache[pk];
      candidates.add(
        MentionCandidate(
          pubkey: pk,
          displayName: profile?.displayName?.trim().isNotEmpty == true
              ? profile!.displayName!.trim()
              : agent.displayName,
          secondaryLabel: profile?.nip05Handle,
          avatarUrl: profile?.avatarUrl ?? agent.avatarUrl,
          isAgent: true,
          isMember: false,
          ownerPubkey: currentPubkey,
        ),
      );
    }
  }

  final directoryPubkeys = <String>{};
  final sharedAgentPubkeys = <String>{};
  for (final agent in relayAgents) {
    directoryPubkeys.add(agent.pubkey);
    if (agentIsSharedWithUser(agent, sharedChannelIds, currentPubkey)) {
      sharedAgentPubkeys.add(agent.pubkey);
    }
  }

  for (final agent in relayAgents) {
    final pk = agent.pubkey;
    if (seen.contains(pk)) continue;
    if (!sharedAgentPubkeys.contains(pk)) continue;
    final profile = userCache[pk];
    final ownerPubkey = ownerByAgentPubkey[pk] ?? profile?.ownerPubkey;
    if (unlistedOwnAgent(pk, ownerPubkey)) continue;
    seen.add(pk);
    candidates.add(
      MentionCandidate(
        pubkey: pk,
        displayName: profile?.displayName?.trim().isNotEmpty == true
            ? profile!.displayName!.trim()
            : agent.displayName,
        secondaryLabel: profile?.nip05Handle,
        avatarUrl: profile?.avatarUrl,
        isAgent: true,
        isMember: false,
        ownerPubkey: ownerPubkey,
      ),
    );
  }

  for (final profile in searchResults) {
    final pk = profile.pubkey.toLowerCase();
    if (seen.contains(pk)) continue;
    final ownerPubkey = ownerByAgentPubkey[pk] ?? profile.ownerPubkey;
    final isAgent = ownerPubkey != null || directoryPubkeys.contains(pk);
    if (isAgent) {
      // Mirrors desktop's `shouldHideAgentFromMentions` for non-member
      // agents: show only when invocable. Invocable = owned by the current
      // user (desktop's managed-agent semantics, derived here from the
      // verified NIP-OA owner) or shared via the relay agent directory.
      final ownedByCurrentUser =
          currentLower != null && ownerPubkey?.toLowerCase() == currentLower;
      if (!ownedByCurrentUser && !sharedAgentPubkeys.contains(pk)) {
        continue;
      }
      if (unlistedOwnAgent(pk, ownerPubkey)) continue;
    }
    seen.add(pk);
    candidates.add(
      MentionCandidate(
        pubkey: pk,
        displayName: profile.displayName?.trim().isNotEmpty == true
            ? profile.displayName!.trim()
            : null,
        secondaryLabel: profile.nip05Handle,
        avatarUrl: profile.avatarUrl,
        isAgent: isAgent,
        isMember: false,
        ownerPubkey: ownerPubkey,
      ),
    );
  }

  return candidates;
}
