part of '../channel_detail_page_test.dart';

void bubbleLayoutTests() {
  testWidgets('channel groups keep one avatar on the last message', (
    tester,
  ) async {
    final notifier = _FakeMessagesNotifier([
      _textMsg(id: 'first', pubkey: 'alice', content: 'First'),
      _textMsg(id: 'last', pubkey: 'alice', content: 'Last', createdAt: 1010),
    ]);
    await tester.pumpWidget(
      _buildTestable(messages: const [], messagesNotifier: notifier),
    );
    await tester.pumpAndSettle();
    final first = find.byKey(const ValueKey('message-row-first'));
    final last = find.byKey(const ValueKey('message-row-last'));
    expect(
      find.descendant(of: first, matching: find.byType(AvatarImage)),
      findsNothing,
    );
    expect(
      find.descendant(of: last, matching: find.byType(AvatarImage)),
      findsOneWidget,
    );
    expect(find.byKey(const ValueKey('message-author-first')), findsOneWidget);
    expect(find.byKey(const ValueKey('message-author-last')), findsNothing);
    expect(
      find.byWidgetPredicate(
        (widget) =>
            widget.key is ValueKey<String> &&
            (widget.key! as ValueKey<String>).value.startsWith(
              'message-text-bubble-',
            ),
      ),
      findsNWidgets(2),
    );
  });

  testWidgets('live appends move the group avatar and author changes end it', (
    tester,
  ) async {
    final first = _textMsg(id: 'first', pubkey: 'alice', content: 'First');
    final next = _textMsg(
      id: 'next',
      pubkey: 'alice',
      content: 'Next',
      createdAt: 1010,
    );
    final other = _textMsg(
      id: 'other',
      pubkey: 'bob',
      content: 'Other',
      createdAt: 1020,
    );
    final notifier = _FakeMessagesNotifier([first]);
    await tester.pumpWidget(
      _buildTestable(messages: const [], messagesNotifier: notifier),
    );
    await tester.pumpAndSettle();
    Finder avatar(String id) => find.descendant(
      of: find.byKey(ValueKey('message-row-$id')),
      matching: find.byType(AvatarImage),
    );
    expect(avatar('first'), findsOneWidget);
    notifier.setMessages([first, next]);
    await tester.pumpAndSettle();
    expect(avatar('first'), findsNothing);
    expect(avatar('next'), findsOneWidget);
    notifier.setMessages([first, next, other]);
    await tester.pumpAndSettle();
    expect(avatar('next'), findsOneWidget);
    expect(avatar('other'), findsOneWidget);
    notifier.setMessages([first]);
    await tester.pumpAndSettle();
    expect(avatar('first'), findsOneWidget);
  });

  for (final gap in [301, 86400]) {
    testWidgets('author groups end across a $gap second gap', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(id: 'first', pubkey: 'alice', content: 'First'),
            _textMsg(
              id: 'next',
              pubkey: 'alice',
              content: 'Next',
              createdAt: 1000 + gap,
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      for (final id in ['first', 'next']) {
        final row = find.byKey(ValueKey('message-row-$id'));
        expect(
          find.descendant(of: row, matching: find.byType(AvatarImage)),
          findsOneWidget,
        );
        expect(find.byKey(ValueKey('message-author-$id')), findsOneWidget);
      }
    });
  }

  testWidgets(
    'thread replies use the same bubble and group-end avatar layout',
    (tester) async {
      final events = [
        _textMsg(id: 'root', pubkey: 'bob', content: 'Root'),
        _textMsg(
          id: 'first',
          pubkey: 'alice',
          content: 'First reply',
          createdAt: 1010,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
        _textMsg(
          id: 'last',
          pubkey: 'alice',
          content: 'Last reply',
          createdAt: 1020,
          extraTags: const [
            ['e', 'root', '', 'reply'],
          ],
        ),
      ];
      final messages = formatTimeline(events);
      await tester.pumpWidget(
        _buildTestable(
          messages: events,
          home: ThreadDetailPage(
            threadHead: messages.firstWhere((m) => m.id == 'root'),
            allMessages: messages,
            channelId: _channelId,
            currentPubkey: 'me',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();
      final first = find.byKey(const ValueKey('thread-message-row-first'));
      final last = find.byKey(const ValueKey('thread-message-row-last'));
      expect(
        find.descendant(of: first, matching: find.byType(AvatarImage)),
        findsNothing,
      );
      expect(
        find.descendant(of: last, matching: find.byType(AvatarImage)),
        findsOneWidget,
      );
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget.key is ValueKey<String> &&
              (widget.key! as ValueKey<String>).value.startsWith(
                'message-text-bubble-',
              ),
        ),
        findsNWidgets(3),
      );
    },
  );
}
