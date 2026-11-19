part of '../channel_detail_page_test.dart';

void inlineActivityTests() {
  testWidgets(
    'quiet activity expands in place, receives updates and keeps public replies',
    (tester) async {
      tester.view.physicalSize = const Size(430, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      NostrEvent update(String id, String content, int time) => _textMsg(
        id: id,
        pubkey: 'alice',
        content: content,
        createdAt: time,
        extraTags: const [
          ['audience', 'agents'],
        ],
      );
      final first = update('activity-one', 'Inspecting the source', 1001);
      final second = update('activity-two', 'Checking the behavior', 1002);
      final messages = _FakeMessagesNotifier([first, second]);
      final typing = _FakeTypingNotifier(const [
        TypingEntry(pubkey: 'alice', expiresAtMs: 9999999999999),
      ]);
      await tester.pumpWidget(
        _buildTestable(
          messages: [first, second],
          messagesNotifier: messages,
          typingNotifier: typing,
          disableAnimations: true,
          textScaler: const TextScaler.linear(2),
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Inspecting the source'), findsNothing);
      expect(find.text('Checking the behavior'), findsNothing);
      final show = find.bySemanticsLabel(RegExp(r'^Show activity from '));
      expect(show, findsOneWidget);
      await _capturePresence(tester, 'activity-collapsed');
      await tester.tap(show);
      await tester.pumpAndSettle();
      expect(find.text('Inspecting the source'), findsOneWidget);
      expect(find.text('Checking the behavior'), findsOneWidget);
      expect(find.text('Live activity is not connected.'), findsOneWidget);
      await _capturePresence(tester, 'activity-expanded');
      final third = update('activity-three', 'Tests passed', 1003);
      messages.setMessages([first, second, third]);
      await tester.pumpAndSettle();
      expect(find.text('Tests passed'), findsOneWidget);
      await tester.ensureVisible(
        find.bySemanticsLabel(RegExp(r'^Collapse activity from ')),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.bySemanticsLabel(RegExp(r'^Collapse activity from ')),
      );
      await tester.pumpAndSettle();
      expect(find.text('Tests passed'), findsNothing);
      typing.setEntries([]);
      messages.setMessages([
        first,
        second,
        third,
        _textMsg(
          id: 'done',
          pubkey: 'alice',
          content: 'Ready for you',
          createdAt: 1004,
        ),
      ]);
      await tester.pumpAndSettle();
      expect(find.text('Ready for you'), findsOneWidget);
      expect(find.text('Activity · 3'), findsOneWidget);
      await tester.tap(find.bySemanticsLabel(RegExp(r'^Show activity from ')));
      await tester.pumpAndSettle();
      expect(find.text('Tests passed'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('thread activity folds and a direct link reveals its history', (
    tester,
  ) async {
    final root = _textMsg(
      id: threadIdRoot,
      pubkey: 'self',
      content: 'Please investigate',
      createdAt: 1000,
    );
    final first = _textMsg(
      id: threadIdA,
      pubkey: 'alice',
      content: 'Thread investigation',
      createdAt: 1001,
      extraTags: const [
        ['e', threadIdRoot, '', 'root'],
        ['e', threadIdRoot, '', 'reply'],
        ['audience', 'agents'],
      ],
    );
    final second = _textMsg(
      id: threadIdR1,
      pubkey: 'alice',
      content: 'Thread evidence',
      createdAt: 1002,
      extraTags: const [
        ['e', threadIdRoot, '', 'root'],
        ['e', threadIdRoot, '', 'reply'],
        ['audience', 'agents'],
      ],
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: [root, first, second],
        threadReplies: {
          threadIdRoot: [first, second],
        },
        disableAnimations: true,
        users: const {
          'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
        },
      ),
    );
    await tester.pumpAndSettle();
    Future<void> open({String? target}) async {
      unawaited(
        Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: formatTimeline([root]).first,
              allMessages: formatTimeline([root, first, second]),
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
              initialMessageId: target,
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    await open();
    expect(find.text('Thread investigation'), findsNothing);
    await tester.tap(find.bySemanticsLabel(RegExp(r'^Show activity from ')));
    await tester.pumpAndSettle();
    expect(find.text('Thread evidence'), findsOneWidget);
    await tester.tap(
      find.bySemanticsLabel(RegExp(r'^Collapse activity from ')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Thread evidence'), findsNothing);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    await open(target: threadIdR1);
    expect(find.text('Thread evidence'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
