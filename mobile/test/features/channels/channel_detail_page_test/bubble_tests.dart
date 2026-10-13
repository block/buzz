part of '../channel_detail_page_test.dart';

void bubbleLayoutTests() {
  testWidgets('single-line activity icon and author share a vertical center', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _systemMsg(
            id: 'single-line',
            payload: {'type': 'member_left', 'actor': 'alice'},
          ),
        ],
        users: {
          'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
        },
      ),
    );
    await tester.pumpAndSettle();
    final card = find.byKey(const ValueKey('activity-card-single-line'));
    final icon = find.descendant(of: card, matching: find.byType(Icon));
    final text = find.byKey(const ValueKey('system-message-author-alice'));
    expect(tester.getCenter(icon).dy, closeTo(tester.getCenter(text).dy, 0.01));
    expect(
      tester.getTopLeft(text).dx - tester.getTopRight(icon).dx,
      closeTo(4, 0.01),
    );
  });

  testWidgets(
    'keeps centered activities timestamp-free at accessibility text sizes',
    (tester) async {
      tester.view.physicalSize = const Size(240, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'sys-accessible',
              payload: {'type': 'member_left', 'actor': 'alice'},
              createdAt:
                  DateTime(2026, 7, 28, 12, 34).millisecondsSinceEpoch ~/ 1000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
          textScaler: const TextScaler.linear(3),
        ),
      );
      await tester.pumpAndSettle();

      final timestampFinder = find.byKey(
        const ValueKey('system-message-timestamp-alice'),
      );
      expect(timestampFinder, findsNothing);
      final card = find.byKey(const ValueKey('activity-card-sys-accessible'));
      final icon = find.descendant(of: card, matching: find.byType(Icon));
      expect(
        tester.getCenter(icon).dy,
        closeTo(
          tester
              .getCenter(
                find.byKey(const ValueKey('system-message-author-alice')),
              )
              .dy,
          0.01,
        ),
      );
      expect(tester.takeException(), isNull);
    },
  );

  for (final thread in [false, true]) {
    testWidgets(
      'status bar tap reaches oldest content, thread=$thread',
      (tester) async {
        final events = [
          _textMsg(
            id: 'top-root',
            pubkey: 'bob',
            content: 'Oldest content',
            createdAt: 900,
          ),
          for (var index = 0; index < 60; index++)
            _textMsg(
              id: 'top-$index',
              pubkey: 'alice',
              content: 'Message $index',
              createdAt: 1000 + index,
              extraTags: thread
                  ? const [
                      ['e', 'top-root', '', 'reply'],
                    ]
                  : const [],
            ),
        ];
        final messages = formatTimeline(events);
        await tester.pumpWidget(
          _buildTestable(
            messages: events,
            home: thread
                ? ThreadDetailPage(
                    threadHead: messages.firstWhere((m) => m.id == 'top-root'),
                    allMessages: messages,
                    channelId: _channelId,
                    currentPubkey: 'me',
                    isMember: true,
                    isArchived: false,
                  )
                : null,
          ),
        );
        await tester.pumpAndSettle();
        final list = tester.widget<ScrollablePositionedList>(
          find.byKey(
            ValueKey(thread ? 'thread-message-list' : 'channel-message-list'),
          ),
        );
        list.itemScrollController!.jumpTo(index: thread ? 55 : 0);
        await tester.pumpAndSettle();
        expect(find.text('Oldest content').hitTestable(), findsNothing);
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'flutter/status_bar',
          const JSONMethodCodec().encodeMethodCall(
            const MethodCall('handleScrollToTop'),
          ),
          (_) {},
        );
        await tester.pumpAndSettle();
        expect(find.text('Oldest content').hitTestable(), findsOneWidget);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );
  }

  testWidgets('newest short message keeps the same group gap', (tester) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          for (var index = 0; index < 3; index++)
            _textMsg(
              id: 'gap-$index',
              pubkey: 'alice',
              content: 'Short',
              createdAt: 1000 + index,
            ),
        ],
      ),
    );
    await tester.pumpAndSettle();
    Rect bubble(int index) => tester.getRect(
      find.descendant(
        of: find.byKey(ValueKey('message-row-gap-$index')),
        matching: find.byKey(const ValueKey('message-text-bubble-0')),
      ),
    );
    final firstGap = bubble(1).top - bubble(0).bottom;
    final newestGap = bubble(2).top - bubble(1).bottom;
    expect(newestGap, closeTo(firstGap, 0.01));
  });

  testWidgets('newest short thread reply keeps the same group gap', (
    tester,
  ) async {
    final events = [
      _textMsg(id: 'root', pubkey: 'bob', content: 'Root', createdAt: 900),
      for (var index = 0; index < 3; index++)
        _textMsg(
          id: 'gap-$index',
          pubkey: 'alice',
          content: 'Short',
          createdAt: 1000 + index,
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
    Rect bubble(int index) => tester.getRect(
      find.descendant(
        of: find.byKey(ValueKey('thread-message-row-gap-$index')),
        matching: find.byKey(const ValueKey('message-text-bubble-0')),
      ),
    );
    expect(
      bubble(2).top - bubble(1).bottom,
      closeTo(bubble(1).top - bubble(0).bottom, 0.01),
    );
  });

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
