part of '../channel_detail_page_test.dart';

void agentActivityTests() {
  const scout = 'scout', helper = 'helper';
  const users = {
    'self': UserProfile(pubkey: 'self', displayName: 'Me'),
    scout: UserProfile(pubkey: scout, displayName: 'Scout'),
    helper: UserProfile(pubkey: helper, displayName: 'Helper'),
  };
  final quiet = ['audience', 'agents'];
  NostrEvent step(String id, String pubkey, int at, {String? root}) => _textMsg(
    id: id,
    pubkey: pubkey,
    content: 'Step $id',
    createdAt: at,
    extraTags: [
      quiet,
      if (root != null) ['e', root, '', 'reply'],
    ],
  );
  Finder row(String id) => find.byKey(ValueKey('message-row-$id'));
  Finder threadRow(String id) => find.byKey(ValueKey('thread-message-$id'));

  testWidgets('agent-only messages fold into one expandable preview', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        users: users,
        textScaler: const TextScaler.linear(1.5),
        messages: [
          _textMsg(id: 'ask', pubkey: 'self', content: 'Please look'),
          step('s1', scout, 1001),
          step('s2', helper, 1002),
          step('s3', scout, 1003),
          _textMsg(
            id: 'answer',
            pubkey: scout,
            content: 'All done',
            createdAt: 1004,
          ),
          step('later', scout, 1005),
        ],
      ),
    );
    await tester.pumpAndSettle();

    // Collapsed: one preview per run naming the run's first agent and its
    // latest step; no step renders, and the public reply keeps its author.
    for (final id in ['s1', 's2', 's3', 'later']) {
      expect(row(id), findsNothing);
    }
    final preview = find.byKey(const ValueKey('activity-preview-s1'));
    expect(preview, findsOneWidget);
    expect(
      find.byKey(const ValueKey('activity-preview-later')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: preview, matching: find.textContaining('Step s3')),
      findsOneWidget,
    );
    expect(find.text('Scout: '), findsOneWidget);
    expect(row('answer'), findsOneWidget);
    expect(find.byKey(const ValueKey('message-author-answer')), findsOneWidget);
    expect(
      find.bySemanticsLabel(RegExp('Show activity history')),
      findsNWidgets(2),
    );
    expect(tester.getSize(preview).height, greaterThanOrEqualTo(Grid.xl));

    // Expanded: every step of that run, then one Collapse control where the
    // preview was.
    await tester.tap(preview);
    await tester.pumpAndSettle();
    for (final id in ['s1', 's2', 's3']) {
      expect(row(id), findsOneWidget);
    }
    expect(row('later'), findsNothing);
    final collapse = find.byKey(const ValueKey('activity-collapse-s1'));
    expect(collapse.hitTestable(), findsOneWidget);
    expect(preview, findsNothing);
    expect(tester.getSize(collapse).height, greaterThanOrEqualTo(Grid.xl));

    await tester.tap(collapse);
    await tester.pumpAndSettle();
    expect(row('s1'), findsNothing);
    expect(preview, findsOneWidget);
    semantics.dispose();
  });

  testWidgets('a link to a folded step opens its run', (tester) async {
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        users: users,
        initialMessageId: 's2',
        messages: [
          _textMsg(id: 'ask', pubkey: 'self', content: 'Please look'),
          step('s1', scout, 1001),
          step('s2', helper, 1002),
          step('s3', scout, 1003),
        ],
      ),
    );
    await tester.pumpAndSettle();
    expect(row('s2'), findsOneWidget);
    expect(find.byKey(const ValueKey('activity-collapse-s1')), findsOneWidget);
  });

  testWidgets('thread steps fold until expanded', (tester) async {
    final head = formatTimeline([
      _textMsg(id: threadIdRoot, pubkey: 'self', content: 'Thread question'),
    ]).first;
    final replies = [
      step(threadIdA, scout, 1001, root: threadIdRoot),
      step(threadIdMid, scout, 1002, root: threadIdRoot),
      _textMsg(
        id: threadIdR1,
        pubkey: scout,
        content: 'Thread answer',
        createdAt: 1003,
        extraTags: [
          ['e', threadIdRoot, '', 'reply'],
        ],
      ),
    ];
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        users: users,
        threadReplies: {threadIdRoot: replies},
        messages: const [],
        home: ThreadDetailPage(
          threadHead: head,
          allMessages: [head, ...formatTimeline(replies)],
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pumpAndSettle();
    final preview = find.byKey(ValueKey('activity-preview-$threadIdA'));
    expect(preview, findsOneWidget);
    expect(threadRow(threadIdA), findsNothing);
    expect(threadRow(threadIdMid), findsNothing);
    expect(threadRow(threadIdR1), findsOneWidget);

    await tester.tap(preview);
    await tester.pumpAndSettle();
    expect(threadRow(threadIdA), findsOneWidget);
    expect(threadRow(threadIdMid), findsOneWidget);
    await tester.tap(find.byKey(ValueKey('activity-collapse-$threadIdA')));
    await tester.pumpAndSettle();
    expect(threadRow(threadIdMid), findsNothing);
  });
}
