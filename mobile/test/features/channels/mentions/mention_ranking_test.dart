import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/mentions/mention_ranking.dart';

// Ported from desktop/src/features/messages/lib/mentionRanking.test.mjs
// (persona cases omitted — personas are desktop-only).

final channelBrainPubkey = '1' * 64;
final otherBrainPubkey = '2' * 64;

MentionCandidate candidate({
  String? displayName = 'Brain',
  String? secondaryLabel,
  bool isAgent = false,
  bool isMember = false,
  String? pubkey,
}) {
  return MentionCandidate(
    pubkey: pubkey ?? otherBrainPubkey,
    displayName: displayName,
    secondaryLabel: secondaryLabel,
    isAgent: isAgent,
    isMember: isMember,
  );
}

List<String> rankedPubkeys(
  List<MentionCandidate> candidates, [
  String query = 'brain',
]) {
  return [
    for (final ranked in rankMentionCandidates(candidates, query))
      ranked.pubkey,
  ];
}

void main() {
  test('channel members outrank people and other agents', () {
    final remoteAgent = candidate(isAgent: true, pubkey: otherBrainPubkey);
    final person = candidate(pubkey: '6' * 64);
    final channelMember = candidate(
      isAgent: true,
      isMember: true,
      pubkey: channelBrainPubkey,
    );

    expect(rankedPubkeys([remoteAgent, person, channelMember]), [
      channelBrainPubkey,
      '6' * 64,
      otherBrainPubkey,
    ]);
  });

  test('exact and prefix quality sort within the channel-member group', () {
    final wordPrefixMember = candidate(
      displayName: 'The Brain',
      isMember: true,
      pubkey: '3' * 64,
    );
    final exactMember = candidate(
      displayName: 'Brain',
      isMember: true,
      pubkey: channelBrainPubkey,
    );
    final prefixMember = candidate(
      displayName: 'Brainiac',
      isMember: true,
      pubkey: '4' * 64,
    );

    expect(rankedPubkeys([wordPrefixMember, exactMember, prefixMember]), [
      channelBrainPubkey,
      '4' * 64,
      '3' * 64,
    ]);
  });

  test('matching secondary labels participate in ranking', () {
    final memberByHandle = candidate(
      displayName: 'Acme Bot',
      secondaryLabel: 'brain@example.com',
      isMember: true,
      pubkey: channelBrainPubkey,
    );
    final nonMemberName = candidate(
      displayName: 'Brain',
      pubkey: otherBrainPubkey,
    );

    expect(rankedPubkeys([nonMemberName, memberByHandle]), [
      channelBrainPubkey,
      otherBrainPubkey,
    ]);
  });

  test('non-matching candidates are dropped', () {
    final match = candidate(displayName: 'Brain', pubkey: channelBrainPubkey);
    final noMatch = candidate(displayName: 'Pinky', pubkey: '7' * 64);

    expect(rankedPubkeys([match, noMatch]), [channelBrainPubkey]);
  });

  test('pubkey prefix matches when no label matches', () {
    final byPubkey = candidate(displayName: 'Pinky', pubkey: 'abc${'0' * 61}');

    expect(rankedPubkeys([byPubkey], 'abc'), ['abc${'0' * 61}']);
  });

  test('empty query keeps stable order within groups', () {
    final second = candidate(displayName: 'Beta', pubkey: '9' * 64);
    final first = candidate(displayName: 'Alpha', pubkey: '8' * 64);

    expect(rankedPubkeys([second, first], ''), ['9' * 64, '8' * 64]);
  });
}
