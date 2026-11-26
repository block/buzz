part of 'message_actions_test.dart';

void _messageActionsScrollTests() {
  for (final (platform, sameTouch) in [
    (TargetPlatform.iOS, false),
    (TargetPlatform.iOS, true),
    (TargetPlatform.android, false),
    (TargetPlatform.android, true),
  ]) {
    testWidgets(
      'overflowing message actions hand off in both directions: $platform, '
      '${sameTouch ? 'same-touch' : 'new-touch'} swipe dismissal',
      (tester) async {
        debugDefaultTargetPlatformOverride = platform;
        tester.view.physicalSize = const Size(375, 667);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        try {
          await _pumpMessageActionsPopover(
            tester,
            message: _message(rootId: threadIdRootDash1),
            prefs: await _mockPrefs(),
            canManageMessage: true,
            allMessages: [_message()],
            reminderService: _stubReminderService(),
            textScaler: TextScaler.linear(2),
          );
          final body = find
              .descendant(
                of: find.byType(BottomSheet),
                matching: find.byType(Scrollable),
              )
              .first;
          final position = tester.state<ScrollableState>(body).position;
          expect(position.maxScrollExtent, greaterThan(0));
          final sheet = find.byType(BottomSheet);
          final animation = tester
              .widget<BottomSheet>(sheet)
              .animationController!;
          final top = tester.getTopLeft(sheet).dy;
          final viewport = tester.getRect(body);
          final start = Offset(viewport.center.dx, viewport.top + 85);
          final gesture = await tester.startGesture(start);
          await gesture.moveBy(const Offset(0, -24));
          await gesture.moveBy(const Offset(0, -70));
          await tester.pump();
          expect(position.pixels, greaterThan(0));
          expect(animation.value, 1);
          // One update repays content and spends its remainder on the whole route.
          final offset = position.pixels;
          await gesture.moveBy(Offset(0, offset + 40));
          await tester.pump();
          expect(position.pixels, 0);
          expect(tester.getTopLeft(sheet).dy, closeTo(top + 40, 0.1));
          // Reverse without lifting: the same update restores route and scrolls.
          await gesture.moveBy(const Offset(0, -65));
          await tester.pump();
          expect(animation.value, 1);
          expect(position.pixels, closeTo(25, 0.1));
          await gesture.cancel();
          await tester.pumpAndSettle();
          // Lower management actions really are reachable by touch.
          await tester.drag(body, const Offset(0, -900));
          await tester.pumpAndSettle();
          expect(find.text('Delete message').hitTestable(), findsOneWidget);
          position.jumpTo(0);
          await tester.pumpAndSettle();
          await tester.drag(
            find.byKey(const ValueKey('quick-reaction-👍')),
            const Offset(-120, 0),
          );
          await tester.pumpAndSettle();
          expect(animation.value, 1);
          expect(position.pixels, 0);
          // A short slow pull settles open rather than completing the modal future.
          final pull = await tester.startGesture(start);
          await pull.moveBy(const Offset(0, 24));
          await pull.moveBy(const Offset(0, 60));
          await tester.pump();
          expect(animation.value, lessThan(1));
          await tester.pump(const Duration(milliseconds: 200));
          await pull.up();
          await tester.pumpAndSettle();
          expect(animation.value, 1);
          // A system-cancelled touch after a deep pull restores the menu.
          final cancelled = await tester.startGesture(start);
          await cancelled.moveBy(const Offset(0, 24));
          await cancelled.moveBy(Offset(0, tester.getSize(sheet).height * 0.7));
          await tester.pump();
          expect(animation.value, lessThan(0.5));
          await cancelled.cancel();
          await tester.pumpAndSettle();
          expect(animation.value, 1);
          // A second finger on the handle takes the sheet over: lifting the
          // body finger leaves it with the handle, which then settles it open.
          final held = await tester.startGesture(start);
          await held.moveBy(const Offset(0, 24));
          await held.moveBy(const Offset(0, 40));
          final handle = await tester.startGesture(
            tester.getCenter(
              find.byKey(const ValueKey('buzz-sheet-drag-handle')),
            ),
          );
          await handle.moveBy(const Offset(0, 24));
          await handle.moveBy(Offset(0, tester.getSize(sheet).height * 0.6));
          await tester.pump();
          final displaced = animation.value;
          expect(displaced, lessThan(0.5));
          await held.up();
          await tester.pumpAndSettle();
          expect(animation.value, displaced);
          await handle.moveBy(Offset(0, -tester.getSize(sheet).height * 0.5));
          await handle.up();
          await tester.pumpAndSettle();
          expect(animation.value, 1);
          expect(position.pixels, 0);
          // Scroll the list up slightly, then swipe down hard: the finger
          // carries content to the top, pulls the sheet a little and the
          // release velocity dismisses the menu. Either as a new touch after
          // lifting or without lifting at all.
          final touch = await tester.startGesture(
            Offset(start.dx, viewport.bottom - 40),
          );
          for (var i = 1; i <= 6; i++) {
            await touch.moveBy(
              const Offset(0, -8),
              timeStamp: Duration(milliseconds: 40 * i),
            );
          }
          await tester.pump(const Duration(milliseconds: 200));
          var swipe = touch;
          var elapsed = 440;
          if (!sameTouch) {
            await touch.up(timeStamp: const Duration(milliseconds: 440));
            await tester.pumpAndSettle();
            swipe = await tester.startGesture(start);
            elapsed = 0;
          }
          final scrolled = position.pixels;
          expect(scrolled, inExclusiveRange(5, 50));
          for (var i = 1; i <= 8; i++) {
            await swipe.moveBy(
              Offset(0, (scrolled + 80) / 8),
              timeStamp: Duration(milliseconds: elapsed + 8 * i),
            );
          }
          await tester.pump();
          expect(position.pixels, 0);
          expect(animation.value, inExclusiveRange(0.5, 1));
          await swipe.up(timeStamp: Duration(milliseconds: elapsed + 64));
          await tester.pumpAndSettle();
          expect(sheet, findsNothing);
        } finally {
          debugDefaultTargetPlatformOverride = null;
        }
      },
    );
  }

  for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
    testWidgets('overflowing message actions keep keyboard and screen-reader '
        'scrolling: $platform', (tester) async {
      final semantics = tester.ensureSemantics();
      debugDefaultTargetPlatformOverride = platform;
      tester.view.physicalSize = const Size(375, 667);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      try {
        await _pumpMessageActionsPopover(
          tester,
          message: _message(rootId: threadIdRootDash1),
          prefs: await _mockPrefs(),
          canManageMessage: true,
          allMessages: [_message()],
          reminderService: _stubReminderService(),
          textScaler: TextScaler.linear(2),
        );
        final sheet = find.byType(BottomSheet);
        final body = find
            .descendant(of: sheet, matching: find.byType(Scrollable))
            .first;
        final position = tester.state<ScrollableState>(body).position;
        expect(position.maxScrollExtent, greaterThan(0));
        // Page Down from the initial focus on the modal scope scrolls the body.
        await tester.sendKeyEvent(LogicalKeyboardKey.pageDown);
        await tester.pumpAndSettle();
        expect(position.pixels, greaterThan(0));
        // A screen-reader scroll toward the top only scrolls content, even
        // when its fixed step overshoots the top by more than half the sheet.
        position.jumpTo(16);
        await tester.pumpAndSettle();
        final scrollNode = find.semantics
            .byAction(ui.SemanticsAction.scrollDown)
            .evaluate()
            .single;
        tester.binding.performSemanticsAction(
          ui.SemanticsActionEvent(
            type: ui.SemanticsAction.scrollDown,
            viewId: tester.view.viewId,
            nodeId: scrollNode.id,
          ),
        );
        await tester.pumpAndSettle();
        expect(sheet, findsOneWidget);
        expect(position.pixels, 0);
      } finally {
        debugDefaultTargetPlatformOverride = null;
        semantics.dispose();
      }
    });
  }

  for (final outcome in [
    'cancel',
    'unfocused',
    'slow',
    'fast',
    'upward',
    'barrier',
    'back',
    'action',
    'reaction',
    'picker',
  ]) {
    testWidgets('message drag result, draft, focus and popover: $outcome', (
      tester,
    ) async {
      final focus = FocusNode();
      addTearDown(focus.dispose);
      final actions = _RecordingReactionActions();
      final harness = await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: await _mockPrefs(),
        composerFocusNode: focus,
        composerInitiallyFocused: outcome != 'unfocused',
        composerDraft: 'Keep this draft',
        launcherOnNestedRoute: true,
        createChannelActions: (_) => actions,
        emojiDataset: outcome == 'picker'
            ? EmojiDataset.fromJson(
                jsonDecode(File(emojiDataAssetPath).readAsStringSync())
                    as Map<String, dynamic>,
              )
            : null,
      );
      final sheet = find.byType(BottomSheet);
      final animation = tester.widget<BottomSheet>(sheet).animationController!;
      final body = find
          .descendant(of: sheet, matching: find.byType(Scrollable))
          .first;
      final position = tester.state<ScrollableState>(body).position;
      // Exact raw release policy uses the production position and Drag wrapper,
      // independent of platform velocity-estimator heuristics.
      final drag = position.drag(
        DragStartDetails(kind: PointerDeviceKind.touch),
        () {},
      );
      final height = tester.getSize(sheet).height;
      final distance = ['slow', 'upward', 'cancel'].contains(outcome)
          ? height * 0.65
          : 50.0;
      drag.update(
        DragUpdateDetails(
          globalPosition: Offset.zero,
          delta: Offset(0, distance),
          primaryDelta: distance,
        ),
      );
      await tester.pump();
      expect(animation.value, lessThan(1));
      expect(focus.hasFocus, isFalse);
      if (outcome == 'cancel') {
        drag.cancel();
      } else {
        final velocity = outcome == 'fast'
            ? 701.0
            : outcome == 'upward'
            ? -701.0
            : 0.0;
        drag.end(
          DragEndDetails(
            velocity: Velocity(pixelsPerSecond: Offset(0, velocity)),
            primaryVelocity: velocity,
          ),
        );
      }
      await tester.pumpAndSettle();
      if (outcome == 'slow' || outcome == 'fast') {
        expect(sheet, findsNothing);
        expect(focus.hasFocus, isTrue);
      } else {
        expect(animation.value, 1);
        expect(focus.hasFocus, isFalse);
        switch (outcome) {
          case 'barrier':
            await tester.tapAt(const Offset(5, 5));
          case 'back':
            await tester.binding.handlePopRoute();
          case 'action':
            await tester.tap(find.text('Copy text'));
          case 'reaction':
            await tester.tap(find.byKey(const ValueKey('quick-reaction-👍')));
          case 'picker':
            await tester.tap(find.byKey(const ValueKey('quick-reaction-more')));
          default:
            await tester.tapAt(const Offset(5, 5));
        }
        await tester.pumpAndSettle();
        expect(
          focus.hasFocus,
          !['action', 'reaction', 'picker', 'unfocused'].contains(outcome),
        );
        if (outcome == 'reaction') expect(actions.added, ['👍']);
        if (outcome == 'picker') {
          expect(find.text('Copy text'), findsNothing);
          expect(find.byType(BottomSheet), findsOneWidget);
          await tester.tapAt(const Offset(5, 5));
          await tester.pumpAndSettle();
        } else {
          expect(sheet, findsNothing);
        }
      }
      expect(find.text('Keep this draft'), findsOneWidget);
      expect(harness.sourceHidden.value, isFalse);
      expect(
        find.byKey(const ValueKey('message-actions-underlying-page')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-actions-root-page')),
        findsNothing,
      );
    });
  }
}
