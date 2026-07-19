import 'dart:convert';

import '../../../shared/relay/nostr_models.dart';
import '../../profile/user_profile.dart';
import '../channel_management_provider.dart';
import 'mention_ranking.dart';

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
  return '${ownerPubkey.substring(0, 8)}…';
}

/// Assemble the full mention candidate list: channel members first-class,
/// then eligible non-member relay agents, then global user-search results.
/// Mirrors desktop's `useMentions` candidate assembly (minus personas and
/// managed agents, which live in the desktop app's local store; the
/// owner-check on search results below covers their mention-eligibility
/// semantics).
List<MentionCandidate> buildMentionCandidates({
  required List<ChannelMember> members,
  required List<AgentDirectoryEntry> relayAgents,
  required Set<String> sharedChannelIds,
  required Map<String, UserProfile> userCache,
  required Map<String, String> ownerByAgentPubkey,
  List<UserProfile> searchResults = const [],
  String? currentPubkey,
}) {
  final candidates = <MentionCandidate>[];
  final seen = <String>{};

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
    seen.add(pk);
    final profile = userCache[pk];
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
        ownerPubkey: ownerByAgentPubkey[pk] ?? profile?.ownerPubkey,
      ),
    );
  }

  final currentLower = currentPubkey?.toLowerCase();
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
