part of 'channel_detail_page_test.dart';

void timelineCalendarBuilderTests() {
  testWidgets('timeline builders use cached boundaries for every row ending', (
    tester,
  ) async {
    final conversions = <Map<int, int>>[];
    TimelineCalendarDays createCalendar() {
      final counts = <int, int>{};
      conversions.add(counts);
      return TimelineCalendarDays.withConverter((timestamp) {
        counts.update(timestamp, (count) => count + 1, ifAbsent: () => 1);
        // Distinct dates make a stray uncached comparison observable in the
        // rendered row spacing even though these timestamps share a real day.
        return DateTime(2030, 1, timestamp);
      });
    }

    final root = _textMsg(
      id: threadIdThreadDashRoot,
      pubkey: 'alice',
      content: 'Calendar root',
      createdAt: 1000,
    );
    final replies = [
      for (final timestamp in [1060, 1120])
        _textMsg(
          id: 'calendar-reply-$timestamp',
          pubkey: 'bob',
          content: 'Calendar reply $timestamp',
          createdAt: timestamp,
          extraTags: const [
            ['e', threadIdThreadDashRoot, '', 'reply'],
          ],
        ),
    ];
    await tester.pumpWidget(
      _buildTestable(
        calendarDaysFactory: createCalendar,
        messages: [
          root,
          _systemMsg(
            id: 'calendar-system',
            createdAt: 1060,
            payload: {
              'type': 'member_joined',
              'actor': 'alice',
              'target': 'alice',
            },
          ),
          _textMsg(
            id: 'calendar-last',
            pubkey: 'bob',
            content: 'Last',
            createdAt: 1120,
          ),
        ],
        threadReplies: {threadIdThreadDashRoot: replies},
        users: const {
          'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
        },
      ),
    );
    await tester.pumpAndSettle();

    void expectEndSpacing(String rowKey) {
      final padding = tester.widget<Padding>(
        find
            .descendant(
              of: find.byKey(ValueKey(rowKey)),
              matching: find.byType(Padding),
            )
            .first,
      );
      expect(padding.padding.resolve(TextDirection.ltr).bottom, 0);
    }

    expectEndSpacing('message-row-$threadIdThreadDashRoot');
    expectEndSpacing('system-message-row-calendar-system');
    final head = formatTimeline([root]).single;
    Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
      MaterialPageRoute<void>(
        builder: (_) => ThreadDetailPage(
          threadHead: head,
          allMessages: [head],
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expectEndSpacing('thread-message-row-calendar-reply-1060');
    final usedCaches = conversions.where((counts) => counts.isNotEmpty);
    expect(usedCaches, isNotEmpty);
    for (final counts in usedCaches) {
      expect(counts.values, everyElement(1));
    }
  });

  testWidgets('thread shows day dividers when replies cross days', (
    tester,
  ) async {
    final rootCreatedAt =
        DateTime(2025, 1, 1, 12).toUtc().millisecondsSinceEpoch ~/ 1000;
    final nextDayCreatedAt =
        DateTime(2025, 1, 2, 12).toUtc().millisecondsSinceEpoch ~/ 1000;
    final rootEvent = _textMsg(
      id: threadIdThreadDashRoot,
      pubkey: 'alice',
      content: 'Thread root',
      createdAt: rootCreatedAt,
    );
    final replies = [
      _textMsg(
        id: 'reply-same-day',
        pubkey: 'bob',
        content: 'Same day',
        createdAt: rootCreatedAt + 60,
        extraTags: const [
          ['e', threadIdThreadDashRoot, '', 'reply'],
        ],
      ),
      _textMsg(
        id: 'reply-next-day',
        pubkey: 'bob',
        content: 'Next day',
        createdAt: nextDayCreatedAt,
        extraTags: const [
          ['e', threadIdThreadDashRoot, '', 'reply'],
        ],
      ),
    ];

    await tester.pumpWidget(
      _buildTestable(
        messages: [rootEvent],
        threadReplies: {threadIdThreadDashRoot: replies},
        users: {
          'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
        },
      ),
    );
    await tester.pumpAndSettle();

    final threadHead = formatTimeline([rootEvent]).single;
    Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
      MaterialPageRoute<void>(
        builder: (_) => ThreadDetailPage(
          threadHead: threadHead,
          allMessages: [threadHead],
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byType(DayDivider), findsNWidgets(2));
    expect(
      find.descendant(
        of: find.byType(DayDivider),
        matching: find.text(formatDayHeading(rootCreatedAt)),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: find.byType(DayDivider),
        matching: find.text(formatDayHeading(nextDayCreatedAt)),
      ),
      findsOneWidget,
    );
    // The list runs top-down (head first), so tail spacing lives on the list
    // and reply groups carry none.
    final threadList = tester.widget<ScrollablePositionedList>(
      find.byKey(const ValueKey('thread-message-list')),
    );
    expect(threadList.reverse, isFalse);
    final threadComposerDockHeight = tester
        .getSize(find.byKey(const ValueKey('thread-composer-dock')))
        .height;
    expect(threadList.padding!.bottom, Grid.xs + threadComposerDockHeight);
    final newestThreadGroup = tester.widget<Padding>(
      find.byKey(const ValueKey('thread-message-group-reply-next-day')),
    );
    expect(newestThreadGroup.padding, EdgeInsets.zero);

    // The head sits above its replies rather than jammed against the
    // composer, matching desktop's thread panel.
    final headY = tester
        .getTopLeft(
          find.byKey(
            const ValueKey('thread-message-group-$threadIdThreadDashRoot'),
          ),
        )
        .dy;
    final oldestReplyY = tester
        .getTopLeft(
          find.byKey(const ValueKey('thread-message-group-reply-same-day')),
        )
        .dy;
    final newestReplyY = tester
        .getTopLeft(
          find.byKey(const ValueKey('thread-message-group-reply-next-day')),
        )
        .dy;
    expect(headY, lessThan(oldestReplyY));
    expect(oldestReplyY, lessThan(newestReplyY));
    final threadTimestamp = tester.widget<Text>(
      find.byKey(
        const ValueKey('thread-message-timestamp-$threadIdThreadDashRoot'),
      ),
    );
    expect(threadTimestamp.style?.fontSize, 11);
    expect(
      find.descendant(
        of: find.byKey(
          const ValueKey('thread-message-row-$threadIdThreadDashRoot'),
        ),
        matching: find.text('·'),
      ),
      findsNothing,
    );
  });
}
