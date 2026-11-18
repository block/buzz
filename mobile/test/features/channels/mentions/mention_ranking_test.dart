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

  test('a NIP-05 handle is not searchable', () {
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

    expect(rankedPubkeys([nonMemberName, memberByHandle]), [otherBrainPubkey]);
  });

  test('non-matching candidates are dropped', () {
    final match = candidate(displayName: 'Brain', pubkey: channelBrainPubkey);
    final noMatch = candidate(displayName: 'Pinky', pubkey: '7' * 64);

    expect(rankedPubkeys([match, noMatch]), [channelBrainPubkey]);
  });

  test('a public key is not searchable', () {
    final byPubkey = candidate(displayName: 'Pinky', pubkey: 'abc${'0' * 61}');
    final unnamed = candidate(displayName: null, pubkey: 'abd${'0' * 61}');

    expect(rankedPubkeys([byPubkey, unnamed], 'ab'), isEmpty);
  });

  test('empty query orders by label, then key, for any input order', () {
    final second = candidate(displayName: 'Beta', pubkey: '9' * 64);
    final first = candidate(displayName: 'Alpha', pubkey: '8' * 64);

    expect(rankedPubkeys([second, first], ''), ['8' * 64, '9' * 64]);
    expect(rankedPubkeys([first, second], ''), ['8' * 64, '9' * 64]);
  });

  test('the viewer\'s own agent ranks first among equal matches', () {
    final viewer = 'f' * 64;
    final mine = MentionCandidate(
      pubkey: '5' * 64,
      displayName: 'Brain',
      isAgent: true,
      ownerPubkey: viewer,
    );
    final person = candidate(pubkey: '3' * 64);

    expect(
      [
        for (final c in rankMentionCandidates(
          [person, mine],
          'brain',
          viewer: viewer,
        ))
          c.pubkey,
      ],
      ['5' * 64, '3' * 64],
    );
  });

  test('recency and presence order agents of one block', () {
    final a = candidate(isAgent: true, pubkey: '3' * 64);
    final b = candidate(isAgent: true, pubkey: '4' * 64);
    List<String> order({Map<String, int> history = const {}, String? online}) =>
        [
          for (final c in rankMentionCandidates(
            [a, b],
            'brain',
            history: history,
            presence: (key) => key == online ? 'online' : 'unknown',
          ))
            c.pubkey,
        ];

    expect(order(), ['3' * 64, '4' * 64]);
    expect(order(online: '4' * 64), ['4' * 64, '3' * 64]);
    expect(order(history: {'4' * 64: 1}), ['4' * 64, '3' * 64]);
  });

  group('stableMentionRows', () {
    final alice = candidate(displayName: 'Alice', pubkey: '1' * 64);
    final bob = candidate(displayName: 'Bob', pubkey: '2' * 64);
    final carol = candidate(displayName: 'Carol', pubkey: '3' * 64);

    test('shown rows keep their places and new rows join at the bottom', () {
      final result = stableMentionRows([bob, alice], [alice, carol, bob]);
      expect(
        [for (final c in result.rows) c.pubkey],
        ['2' * 64, '1' * 64, '3' * 64],
      );
      expect(result.unavailable, isEmpty);
    });

    test('a row that leaves stays in place, unavailable', () {
      final result = stableMentionRows([alice, bob], [bob]);
      expect([for (final c in result.rows) c.pubkey], ['1' * 64, '2' * 64]);
      expect(result.unavailable, {'1' * 64});
    });

    test('a new list shows at most the limit', () {
      final result = stableMentionRows(const [], [alice, bob, carol], limit: 2);
      expect(result.rows, [alice, bob]);
    });
  });
}
