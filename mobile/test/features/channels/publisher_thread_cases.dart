part of 'channel_detail_page_test.dart';

void publisherThreadTests() {
  const publisher =
      'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
  const relay =
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
  for (final variant in ['direct', 'workflow', 'other-author']) {
    final workflow = variant == 'workflow';
    final allowed = variant != 'other-author';
    testWidgets(
      'publisher channel keeps main composer closed and enables non-member thread replies variant=$variant',
      (tester) async {
        final sender = _RecordingOrdinarySender();
        final channel = Channel(
          id: _channelId,
          name: 'Updates',
          channelType: 'stream',
          visibility: 'open',
          description: '',
          createdBy: publisher,
          createdAt: DateTime.utc(2026),
          memberCount: 1,
          isMember: false,
          messagePublisher: publisher,
        );
        final root = _textMsg(
          id: threadIdRoot,
          pubkey: !allowed
              ? 'other-author'
              : workflow
              ? relay
              : publisher,
          content: 'Publisher update',
          extraTags: workflow
              ? [
                  ['actor', publisher],
                ]
              : [],
        );
        await tester.pumpWidget(
          _buildTestable(
            messages: [root],
            sender: sender,
            channel: channel,
            publisherRoots: {
              root.id: (
                event: root,
                author: publisherMessageAuthor(root, relay),
              ),
            },
            threadReplies: {root.id: []},
          ),
        );
        await tester.pumpAndSettle();
        expect(find.byType(ComposeBar), findsNothing);
        expect(
          find.text('Open a post by the channel publisher to reply'),
          findsOneWidget,
        );
        final context = tester.element(find.byType(ChannelDetailPage));
        final head = formatTimeline([root]).single;
        Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: head,
              allMessages: [head],
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: false,
              isArchived: false,
            ),
          ),
        );
        await tester.pumpAndSettle();
        if (!allowed) {
          expect(find.byType(ComposeBar), findsNothing);
          return;
        }
        expect(find.byType(ComposeBar), findsOneWidget);
        await tester
            .widget<ComposeBar>(find.byType(ComposeBar))
            .onSend('My reply', []);
        expect(sender.sent, ['My reply']);
        expect(sender.parents, [root.id]);
      },
    );
  }

  testWidgets(
    'ordinary nested replies remain usable after the outer root is deleted',
    (tester) async {
      final root = _textMsg(
        id: threadIdRoot,
        pubkey: 'alice',
        content: 'Outer root',
      );
      final child = _textMsg(
        id: threadIdA,
        pubkey: 'bob',
        content: 'Still-existing nested head',
        createdAt: 1001,
        extraTags: const [
          ['e', threadIdRoot, '', 'reply'],
        ],
      );
      final messages = _FakeMessagesNotifier([root, child]);
      final sender = _RecordingOrdinarySender();
      await tester.pumpWidget(
        _buildTestable(
          messages: [root, child],
          messagesNotifier: messages,
          sender: sender,
          threadReplies: {
            root.id: [child],
          },
        ),
      );
      await tester.pumpAndSettle();
      final context = tester.element(find.byType(ChannelDetailPage));
      final timeline = formatTimeline([root, child]);
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: timeline.singleWhere(
              (message) => message.id == child.id,
            ),
            allMessages: timeline,
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(ComposeBar), findsOneWidget);
      messages.setMessages([
        root,
        child,
        _deletion(id: 'outer-root-deleted', targetIds: [root.id]),
      ]);
      await tester.pumpAndSettle();
      expect(find.text('Still-existing nested head'), findsOneWidget);
      expect(
        find.byType(ComposeBar),
        findsOneWidget,
        reason:
            'ordinary channels only require the immediate parent to survive',
      );
      await tester
          .widget<ComposeBar>(find.byType(ComposeBar))
          .onSend('Continue the surviving branch', []);
      expect(sender.parents, [child.id]);
      messages.setMessages([
        root,
        child,
        _deletion(id: 'outer-root-deleted', targetIds: [root.id]),
        _deletion(id: 'nested-head-deleted', targetIds: [child.id]),
      ]);
      await tester.pumpAndSettle();
      expect(
        find.byType(ComposeBar),
        findsNothing,
        reason: 'the relay refuses replies to a deleted parent',
      );
    },
  );
}
