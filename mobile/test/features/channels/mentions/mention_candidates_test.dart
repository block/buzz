import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:buzz/features/channels/mentions/mention_ranking.dart';
import 'package:buzz/shared/agents/owned_agent_directory.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';

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
  test('role-only agent mentions fall back to a compact npub label', () {
    const pubkey =
        'deadbeef00000000000000000000000000000000000000000000000000000000';

    expect(
      mentionNamesWithDirectoryLabels(
        mentionPubkeys: const [pubkey],
        profileMentionNames: const {},
        directoryDisplayNames: const {},
        agentMentionPubkeys: const {pubkey},
      ),
      const {pubkey: 'npub1m6k\u20263kf3'},
    );
  });

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

    test('prefers display name, then handle, then compact npub', () {
      final profiles = {
        ownerPubkey: UserProfile(pubkey: ownerPubkey, displayName: 'Wes'),
      };
      expect(formatOwnerLabel(ownerPubkey, userPubkey, profiles), 'Wes');
      expect(
        formatOwnerLabel(ownerPubkey, userPubkey, const {}),
        'npub1mhw\u2026dmpv',
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

    test('search results add non-member humans, ungated', () {
      final humanPubkey = '1' * 64;
      final candidates = buildMentionCandidates(
        members: [member(memberPubkey)],
        relayAgents: const [],
        sharedChannelIds: const {},
        userCache: const {},
        ownerByAgentPubkey: const {},
        searchResults: [
          UserProfile(pubkey: humanPubkey, displayName: 'Wes Outside'),
        ],
        currentPubkey: userPubkey,
      );

      expect(candidates.map((c) => c.pubkey), [memberPubkey, humanPubkey]);
      final human = candidates.last;
      expect(human.isAgent, isFalse);
      expect(human.isMember, isFalse);
      expect(human.displayName, 'Wes Outside');
    });

    test('offers only owned agents the Agents page lists', () {
      final listed = '2' * 64;
      final unlisted = '4' * 64;
      final unlistedMember = '5' * 64;
      UserProfile own(String pubkey, String name) => UserProfile(
        pubkey: pubkey,
        displayName: name,
        ownerPubkey: userPubkey,
      );
      List<MentionCandidate> build({
        required bool offer,
        bool known = true,
      }) => buildMentionCandidates(
        members: [member(unlistedMember)],
        relayAgents: [
          AgentDirectoryEntry(
            pubkey: unlisted,
            respondTo: 'anyone',
            channelIds: const ['chan-1'],
          ),
        ],
        sharedChannelIds: const {'chan-1'},
        userCache: {
          unlisted: own(unlisted, 'old-app'),
          unlistedMember: own(unlistedMember, 'old-member'),
        },
        ownerByAgentPubkey: const {},
        searchResults: [own(listed, 'raccoon'), own(unlisted, 'old-app')],
        currentPubkey: userPubkey,
        ownedAgents: known
            ? [OwnedAgentDirectoryEntry(pubkey: listed, displayName: 'raccoon')]
            : null,
        offerOwnedAgents: offer,
      );

      // Members always stay; an earlier Buzz's agent outside the channel is
      // offered neither from the directory nor from search.
      for (final offer in [false, true]) {
        final candidates = build(offer: offer);
        expect(candidates.map((c) => c.pubkey), [unlistedMember, listed]);
        expect(candidates.last.isAgent, isTrue);
        expect(candidates.last.isMember, isFalse);
        expect(candidates.last.ownerPubkey, userPubkey);
      }
      // While the list is unknown (loading, failed or offline), no owned
      // agent is held back as unlisted.
      expect(build(offer: true, known: false).map((c) => c.pubkey), [
        unlistedMember,
        unlisted,
        listed,
      ]);
      // Streams and forums offer listed agents before anyone searches.
      final offered = buildMentionCandidates(
        members: const [],
        relayAgents: const [],
        sharedChannelIds: const {},
        userCache: const {},
        ownerByAgentPubkey: const {},
        currentPubkey: userPubkey,
        ownedAgents: [
          OwnedAgentDirectoryEntry(pubkey: listed, displayName: 'raccoon'),
        ],
        offerOwnedAgents: true,
      );
      expect(offered.map((c) => c.displayName), ['raccoon']);
    });

    test('search results hide non-shared agents owned by someone else', () {
      final foreignAgent = '3' * 64;
      final candidates = buildMentionCandidates(
        members: const [],
        relayAgents: const [],
        sharedChannelIds: const {},
        userCache: const {},
        ownerByAgentPubkey: const {},
        searchResults: [
          UserProfile(
            pubkey: foreignAgent,
            displayName: 'stranger-bot',
            ownerPubkey: ownerPubkey,
          ),
        ],
        currentPubkey: userPubkey,
      );

      expect(candidates, isEmpty);
    });

    test('search results show non-owned agents shared via the directory', () {
      final candidates = buildMentionCandidates(
        members: const [],
        relayAgents: [
          AgentDirectoryEntry(
            pubkey: agentPubkey,
            respondTo: 'anyone',
            channelIds: const ['chan-1'],
          ),
        ],
        sharedChannelIds: {'chan-1'},
        userCache: const {},
        ownerByAgentPubkey: const {},
        searchResults: [
          UserProfile(
            pubkey: agentPubkey,
            displayName: 'Helper',
            ownerPubkey: ownerPubkey,
          ),
        ],
        currentPubkey: userPubkey,
      );

      // Already surfaced by the directory pass; the search pass must not
      // duplicate it.
      expect(candidates, hasLength(1));
      expect(candidates.single.pubkey, agentPubkey);
      expect(candidates.single.isAgent, isTrue);
    });

    test('directory-listed agents in search results are agents even without '
        'a verified owner', () {
      final candidates = buildMentionCandidates(
        members: const [],
        relayAgents: [
          AgentDirectoryEntry(
            pubkey: agentPubkey,
            respondTo: 'anyone',
            channelIds: const ['chan-9'],
          ),
        ],
        // Not shared with the user → directory pass skips it; the search
        // pass must still classify it as an agent and hide it.
        sharedChannelIds: const {},
        userCache: const {},
        ownerByAgentPubkey: const {},
        searchResults: [
          UserProfile(pubkey: agentPubkey, displayName: 'Helper'),
        ],
        currentPubkey: userPubkey,
      );

      expect(candidates, isEmpty);
    });

    test('search results never duplicate channel members', () {
      final candidates = buildMentionCandidates(
        members: [member(memberPubkey)],
        relayAgents: const [],
        sharedChannelIds: const {},
        userCache: const {},
        ownerByAgentPubkey: const {},
        searchResults: [
          UserProfile(pubkey: memberPubkey, displayName: 'Member Dup'),
        ],
        currentPubkey: userPubkey,
      );

      expect(candidates, hasLength(1));
      expect(candidates.single.isMember, isTrue);
    });
  });
}
