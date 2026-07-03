import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:buzz/features/profile/user_profile.dart';

final userPubkey = 'a' * 64;
final memberPubkey = 'b' * 64;
final agentPubkey = 'c' * 64;
final ownerPubkey = 'd' * 64;

ChannelMember member(String pubkey, {String role = 'member'}) {
  return ChannelMember(
    pubkey: pubkey,
    role: role,
    joinedAt: DateTime(2024),
    displayName: 'Member',
  );
}

void main() {
  group('agentIsSharedWithUser', () {
    test('anyone-mode agent is shared when a channel overlaps', () {
      final agent = AgentDirectoryEntry(
        pubkey: agentPubkey,
        respondTo: 'anyone',
        channelIds: const ['chan-1'],
      );
      expect(agentIsSharedWithUser(agent, {'chan-1'}, userPubkey), isTrue);
      expect(agentIsSharedWithUser(agent, {'chan-2'}, userPubkey), isFalse);
    });

    test('allowlist-mode agent requires the user on the allowlist', () {
      final agent = AgentDirectoryEntry(
        pubkey: agentPubkey,
        respondTo: 'allowlist',
        respondToAllowlist: [userPubkey],
        channelIds: const ['chan-1'],
      );
      expect(agentIsSharedWithUser(agent, {'chan-1'}, userPubkey), isTrue);
      expect(agentIsSharedWithUser(agent, {'chan-1'}, 'e' * 64), isFalse);
    });
  });

  group('formatOwnerLabel', () {
    test('returns "you" for the current user', () {
      expect(formatOwnerLabel(userPubkey, userPubkey, const {}), 'you');
    });

    test('prefers display name, then handle, then pubkey prefix', () {
      final profiles = {
        ownerPubkey: UserProfile(pubkey: ownerPubkey, displayName: 'Wes'),
      };
      expect(formatOwnerLabel(ownerPubkey, userPubkey, profiles), 'Wes');
      expect(
        formatOwnerLabel(ownerPubkey, userPubkey, const {}),
        '${'d' * 8}\u2026',
      );
    });

    test('returns null without an owner', () {
      expect(formatOwnerLabel(null, userPubkey, const {}), isNull);
    });
  });

  group('buildMentionCandidates', () {
    test('members come first; eligible non-member agents follow', () {
      final candidates = buildMentionCandidates(
        members: [member(memberPubkey), member(userPubkey)],
        relayAgents: [
          AgentDirectoryEntry(
            pubkey: agentPubkey,
            displayName: 'Helper',
            respondTo: 'anyone',
            channelIds: const ['chan-1'],
          ),
        ],
        sharedChannelIds: {'chan-1'},
        userCache: const {},
        ownerByAgentPubkey: {agentPubkey: ownerPubkey},
        currentPubkey: userPubkey,
      );

      expect(candidates.map((c) => c.pubkey).toList(), [
        memberPubkey,
        userPubkey,
        agentPubkey,
      ]);
      expect(candidates.last.isAgent, isTrue);
      expect(candidates.last.isMember, isFalse);
      expect(candidates.last.ownerPubkey, ownerPubkey);
    });

    test('includes the current user (desktop parity)', () {
      final candidates = buildMentionCandidates(
        members: [member(userPubkey)],
        relayAgents: const [],
        sharedChannelIds: const {},
        userCache: const {},
        ownerByAgentPubkey: const {},
        currentPubkey: userPubkey,
      );
      expect(candidates.map((c) => c.pubkey), contains(userPubkey));
    });

    test('excludes non-shared agents and deduplicates member agents', () {
      final candidates = buildMentionCandidates(
        members: [member(agentPubkey, role: 'bot')],
        relayAgents: [
          AgentDirectoryEntry(
            pubkey: agentPubkey,
            respondTo: 'anyone',
            channelIds: const ['chan-1'],
          ),
          AgentDirectoryEntry(
            pubkey: 'f' * 64,
            respondTo: 'anyone',
            channelIds: const ['chan-9'],
          ),
        ],
        sharedChannelIds: {'chan-1'},
        userCache: const {},
        ownerByAgentPubkey: const {},
        currentPubkey: userPubkey,
      );

      expect(candidates, hasLength(1));
      expect(candidates.single.pubkey, agentPubkey);
      expect(candidates.single.isMember, isTrue);
      expect(candidates.single.isAgent, isTrue);
    });
  });
}
