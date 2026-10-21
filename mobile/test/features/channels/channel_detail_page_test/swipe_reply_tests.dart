part of '../channel_detail_page_test.dart';

void swipeReplyTests() {
  for (final thread in [false, true]) {
    testWidgets('right drag opens existing reply destination, thread=$thread', (
      tester,
    ) async {
      final events = [
        _textMsg(
          id: threadIdRoot,
          pubkey: 'alice',
          content: 'Swipe this message\nA second line\nAnd a third line',
        ),
      ];
      final messages = formatTimeline(events);
      await tester.pumpWidget(
        _buildTestable(
          messages: events,
          chatColor: 'magenta',
          home: thread
              ? ThreadDetailPage(
                  threadHead: messages.first,
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
      final row = find.byKey(
        ValueKey(
          '${thread ? 'thread-message-row' : 'message-row'}-$threadIdRoot',
        ),
      );
      final initial = tester.getTopLeft(row);
      final haptics = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'HapticFeedback.vibrate') haptics.add(call);
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      final gesture = await tester.startGesture(tester.getCenter(row));
      await gesture.moveBy(const Offset(25, 0));
      await gesture.moveBy(const Offset(40, 0));
      await tester.pump();
      expect(tester.getTopLeft(row).dx, greaterThan(initial.dx));
      expect(haptics, isEmpty);
      final partialCircle = tester.widget<DecoratedBox>(
        find.byKey(const ValueKey('swipe-reply-circle')),
      );
      expect(
        (partialCircle.decoration as BoxDecoration).color,
        Colors.transparent,
      );
      final ring = tester.widget<CircularProgressIndicator>(
        find.descendant(
          of: find.byKey(const ValueKey('swipe-reply-circle')),
          matching: find.byType(CircularProgressIndicator),
        ),
      );
      expect(ring.strokeWidth, 2);
      expect(ring.value, inExclusiveRange(0, 1));
      await gesture.moveBy(const Offset(45, 0));
      await tester.pump();
      expect(haptics, hasLength(1));
      await gesture.moveBy(const Offset(10, 0));
      await tester.pump();
      expect(haptics, hasLength(1));
      final armedPosition = tester.getTopLeft(row);
      await gesture.moveBy(const Offset(160, 0));
      await tester.pump();
      final resistedPosition = tester.getTopLeft(row);
      expect(resistedPosition.dx, greaterThan(armedPosition.dx));
      expect(resistedPosition.dx, lessThan(armedPosition.dx + 40));
      final circle = find.byKey(const ValueKey('swipe-reply-circle'));
      expect(tester.getRect(circle).left, initial.dx);
      final fill =
          tester.widget<DecoratedBox>(circle).decoration as BoxDecoration;
      // The armed circle uses the chosen message color, not the app accent.
      expect(fill.color, const Color(0xFFCC1F7A));
      expect(find.text('Reply'), findsOneWidget);
      final centerBeforePulse = tester.getCenter(
        find.byKey(const ValueKey('swipe-reply-feedback')),
      );
      await tester.pump(const Duration(milliseconds: 60));
      expect(
        tester.getCenter(find.byKey(const ValueKey('swipe-reply-feedback'))),
        centerBeforePulse,
      );
      expect(tester.getRect(circle).width, greaterThan(Grid.lg));
      await tester.pump(const Duration(milliseconds: 180));
      expect(tester.getRect(circle).width, Grid.lg);
      expect(haptics, hasLength(1));
      final feedback = find.byKey(const ValueKey('swipe-reply-feedback'));
      final content = find.descendant(
        of: row,
        matching: find.byType(MessageContent),
      );
      expect(tester.getCenter(feedback).dy, tester.getCenter(content).dy);
      expect(
        tester.getCenter(find.text('Reply')).dx,
        tester.getCenter(circle).dx,
      );
      expect(
        tester.widget<Text>(find.text('Reply')).style?.color,
        tester.element(circle).colors.onSurfaceVariant,
      );
      // Retreat below readiness without returning to rest, then cross again.
      await gesture.moveBy(const Offset(-210, 0));
      await tester.pump();
      expect(tester.getTopLeft(row).dx, greaterThan(initial.dx));
      await gesture.moveBy(const Offset(45, 0));
      await tester.pump();
      expect(haptics, hasLength(2));
      await tester.pump(const Duration(milliseconds: 60));
      expect(tester.getRect(circle).width, greaterThan(Grid.lg));
      await tester.pump(const Duration(milliseconds: 180));
      expect(tester.getRect(circle).width, Grid.lg);
      await gesture.up();
      await tester.pumpAndSettle();
      expect(find.byType(ThreadDetailPage), findsOneWidget);
      expect(
        tester
            .widget<ThreadDetailPage>(find.byType(ThreadDetailPage))
            .threadHead
            .id,
        threadIdRoot,
      );
      // A thread message must push a reply route too, rather than doing nothing.
      expect(
        Navigator.of(tester.element(find.byType(ThreadDetailPage))).canPop(),
        isTrue,
      );
    });
  }

  for (final movement in [
    const Offset(45, 0),
    const Offset(-100, 0),
    const Offset(0, -100),
  ]) {
    testWidgets('non-reply drag $movement does not navigate', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'swipe-cancel',
              pubkey: 'alice',
              content: 'Keep reading',
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('message-row-swipe-cancel'));
      await tester.drag(row, movement);
      await tester.pumpAndSettle();
      expect(find.byType(ThreadDetailPage), findsNothing);
    });
  }

  for (final reducedMotion in [false, true]) {
    testWidgets('cancelled drag resets feedback, reduced=$reducedMotion', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          disableAnimations: reducedMotion,
          messages: [
            _textMsg(id: 'cancel', pubkey: 'alice', content: 'Cancel drag'),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('message-row-cancel'));
      final origin = tester.getTopLeft(row);
      final gesture = await tester.startGesture(tester.getCenter(row));
      await gesture.moveBy(const Offset(25, 0));
      await gesture.moveBy(const Offset(90, 0));
      await tester.pump();
      final icon = find.byIcon(BuzzIcons.messageSquareReply);
      expect(
        tester
            .widget<Opacity>(
              find.ancestor(of: icon, matching: find.byType(Opacity)).first,
            )
            .opacity,
        1,
      );
      await gesture.cancel();
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(row), origin);
      expect(find.byType(ThreadDetailPage), findsNothing);
    });
  }

  testWidgets('crossing threshold then retreating cancels reply', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _textMsg(id: 'retreat', pubkey: 'alice', content: 'Keep reading'),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final row = find.byKey(const ValueKey('message-row-retreat'));
    final gesture = await tester.startGesture(tester.getCenter(row));
    await gesture.moveBy(const Offset(25, 0));
    await gesture.moveBy(const Offset(90, 0));
    await gesture.moveBy(const Offset(-70, 0));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(find.byType(ThreadDetailPage), findsNothing);
  });
}
