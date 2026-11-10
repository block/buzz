part of 'message_content_test.dart';

void stackJourneyTests() {
  testWidgets(
    'sent and received stacks browse whole cards and open the selected gallery photo',
    (tester) async {
      _setSurfaceSize(tester, const Size(375, 800));
      tester.view.devicePixelRatio = 3;
      tester.view.physicalSize = const Size(1125, 2400);
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final semantics = tester.ensureSemantics();
      const first = 'https://example.com/stack-one.png';
      const second = 'https://example.com/stack-two.png';
      const third = 'https://example.com/stack-three.png';
      for (final own in [false, true]) {
        await tester.pumpWidget(
          _testable(
            MessageContent(
              key: ValueKey(own),
              bubble: true,
              ownMessage: own,
              content: '![one]($first)\n![two]($second)\n![three]($third)',
            ),
          ),
        );
        await tester.pumpAndSettle();
        final stack = find.byKey(const ValueKey('message-media-stack'));
        final spoken = find.byKey(
          const ValueKey('message-media-stack-semantics'),
        );
        expect(
          tester
              .widget<Semantics>(
                find.byKey(const ValueKey('message-media-stack-semantics')),
              )
              .properties
              .value,
          'Photo 1 of 3',
        );
        expect(
          tester
              .getSemantics(spoken)
              .getSemanticsData()
              .hasAction(SemanticsAction.increase),
          isTrue,
        );
        Finder cardClip(int index) => find.descendant(
          of: find.byKey(ValueKey('stack-card:$index')),
          matching: find.byType(ClipRSuperellipse),
        );
        {
          Matrix4 pose(int index) => tester
              .widgetList<Transform>(
                find.descendant(
                  of: find.byKey(ValueKey('stack-card:$index')),
                  matching: find.byType(Transform),
                ),
              )
              .last
              .transform;
          // Three-photo stacks retain each reference's distinct middle and
          // back slots, rather than giving both rear cards one pose.
          expect(pose(1).storage[12], closeTo(own ? 12 : 11, .001));
          expect(pose(1).storage[13], closeTo(-5, .001));
          expect(pose(1).storage[1], greaterThan(0));
          expect(pose(2).storage[12], closeTo(own ? -10 : -11, .001));
          expect(pose(2).storage[13], closeTo(4, .001));
          expect(pose(2).storage[1], lessThan(0));
        }
        final resting = tester.getRect(cardClip(0));
        final viewport = tester.getRect(stack);
        final countLabel = find.byKey(
          const ValueKey('message-media-stack-count'),
        );
        final countBefore = tester.getRect(countLabel);
        expect(find.text('3 photos'), findsOneWidget);
        expect(
          find.byKey(const ValueKey('message-media-stack-counter')),
          findsNothing,
        );
        expect(
          own ? viewport.right - resting.right : resting.left - viewport.left,
          closeTo(Grid.xs + Grid.half + Grid.twelve, 1),
        );
        // Sample the complete path, including either side of the layer swap.
        final sweep = await tester.startGesture(tester.getCenter(stack));
        await sweep.moveBy(const Offset(-20, 0));
        for (var step = 1; step < 100; step++) {
          await sweep.moveBy(const Offset(-1.4, 0));
          await tester.pump();
          for (final element
              in find
                  .descendant(
                    of: stack,
                    matching: find.byType(ClipRSuperellipse),
                  )
                  .evaluate()) {
            final box = element.renderObject! as RenderBox;
            for (final corner in [
              Offset.zero,
              Offset(box.size.width, 0),
              Offset(0, box.size.height),
              Offset(box.size.width, box.size.height),
            ]) {
              expect(
                viewport.contains(box.localToGlobal(corner)),
                isTrue,
                reason:
                    'phase ${step / 100}: ${box.localToGlobal(corner)} outside $viewport',
              );
            }
          }
        }
        await sweep.moveBy(const Offset(138.6, 0));
        await sweep.up();
        await tester.pumpAndSettle();
        final drag = await tester.startGesture(tester.getCenter(stack));
        await drag.moveBy(const Offset(-20, 0));
        await drag.moveBy(const Offset(-42, 0));
        await tester.pump();
        expect(
          tester.getRect(countLabel),
          countBefore,
          reason: 'Photo count remains stationary during the swipe',
        );

        expect(find.byKey(const ValueKey('stack-layer-blend')), findsNothing);
        await drag.moveBy(const Offset(-28, 0));
        await tester.pump();
        final arriving = tester.getRect(cardClip(1));
        // The solid cards meet at their edges when their paint order changes.
        expect((arriving.center.dx - resting.center.dx).abs(), greaterThan(12));
        final departing = tester.getRect(cardClip(0));
        // The tuned rotation slightly narrows the projected outgoing card.
        expect(departing.width, greaterThan(resting.width * .45));
        expect(departing.width, lessThan(resting.width * .85));
        expect(arriving.width, greaterThan(resting.width * .6));
        expect(departing.intersect(arriving).width, lessThan(2));
        expect(find.byKey(const ValueKey('stack-layer-blend')), findsNothing);
        expect(
          find.descendant(of: stack, matching: find.byType(ClipPath)),
          findsNothing,
        );
        // Every photo retains its own clip while the two front cards exchange.
        final clips = find.descendant(
          of: stack,
          matching: find.byType(ClipRSuperellipse),
        );
        expect(clips, findsNWidgets(3));
        for (final clip in tester.widgetList<ClipRSuperellipse>(clips)) {
          expect(
            clip.borderRadius,
            BorderRadius.circular(Radii.container + Grid.xxs - Grid.half),
          );
        }
        final stage = tester.getRect(stack);
        for (final element in clips.evaluate()) {
          final box = element.renderObject! as RenderBox;
          for (final corner in [
            Offset.zero,
            Offset(box.size.width, 0),
            Offset(0, box.size.height),
            Offset(box.size.width, box.size.height),
          ]) {
            expect(
              stage.contains(box.localToGlobal(corner)),
              isTrue,
              reason: '${box.localToGlobal(corner)} outside $stage',
            );
          }
        }
        await drag.moveBy(const Offset(-28, 0));
        await tester.pump();
        expect(find.byKey(const ValueKey('stack-layer-blend')), findsNothing);
        await drag.up();
        await tester.pumpAndSettle();
        expect(
          tester
              .widget<Semantics>(
                find.byKey(const ValueKey('message-media-stack-semantics')),
              )
              .properties
              .value,
          'Photo 2 of 3',
        );
        // A canceled touch that never became a drag must not restore an old
        // drag origin after a gallery changes the selected photo.
        final canceledTouch = await tester.startGesture(
          tester.getCenter(stack),
        );
        await canceledTouch.cancel();
        await tester.pumpAndSettle();
        expect(
          tester.widget<Semantics>(spoken).properties.value,
          'Photo 2 of 3',
        );
        await tester.tap(stack);
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('message-media-image-viewer')),
          findsOneWidget,
        );
        expect(
          tester.widget<PageIndicator>(find.byType(PageIndicator)).selected,
          1,
        );
        final viewer = tester.widget<MediaImageViewerPage>(
          find.byType(MediaImageViewerPage),
        );
        final preview = viewer.galleryItems![1];
        final cardImage = tester.widget<MediaImage>(
          find
              .descendant(
                skipOffstage: false,
                of: find.byKey(
                  const ValueKey('stack-card:1'),
                  skipOffstage: false,
                ),
                matching: find.byType(MediaImage, skipOffstage: false),
              )
              .first,
        );
        expect(cardImage.decodeWidth, lessThanOrEqualTo(210));
        expect(preview.previewDecodeWidth, cardImage.decodeWidth);
        final resized = preview.preloadProvider! as ResizeImage;
        expect(resized.width, (cardImage.decodeWidth! * 3).ceil());
        // Page in the full-screen gallery, close, then reopen the same photo.
        await tester.drag(find.byType(PageView), const Offset(-350, 0));
        await tester.pumpAndSettle();
        expect(
          tester.widget<PageIndicator>(find.byType(PageIndicator)).selected,
          2,
        );
        await tester.tap(
          find.byKey(const ValueKey('message-media-image-viewer-close')),
        );
        await tester.pumpAndSettle();
        expect(
          tester.widget<Semantics>(spoken).properties.value,
          'Photo 3 of 3',
        );
        await tester.tap(stack);
        await tester.pumpAndSettle();
        expect(
          tester.widget<PageIndicator>(find.byType(PageIndicator)).selected,
          2,
        );
        await tester.drag(find.byType(PageView), const Offset(350, 0));
        await tester.pumpAndSettle();
        // System back must preserve selection as well as the Close control.
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(
          tester.widget<Semantics>(spoken).properties.value,
          'Photo 2 of 3',
        );
        await tester.drag(stack, const Offset(100, 0));
        await tester.pumpAndSettle();
        expect(
          tester
              .widget<Semantics>(
                find.byKey(const ValueKey('message-media-stack-semantics')),
              )
              .properties
              .value,
          'Photo 1 of 3',
        );
        final reverse = await tester.startGesture(tester.getCenter(stack));
        await reverse.moveBy(const Offset(-20, 0));
        await reverse.moveBy(const Offset(-65, 0));
        await tester.pump();
        await reverse.moveBy(const Offset(65, 0));
        await reverse.up();
        await tester.pumpAndSettle();
        expect(
          tester
              .widget<Semantics>(
                find.byKey(const ValueKey('message-media-stack-semantics')),
              )
              .properties
              .value,
          'Photo 1 of 3',
        );
        expect(
          find.byKey(const ValueKey('message-media-image-viewer')),
          findsNothing,
        );
      }
      // Larger collections keep their rear cards quiet rather than multiplying
      // the fan angle with every additional photo.
      for (final own in [false, true]) {
        await tester.pumpWidget(
          _testable(
            MessageContent(
              key: ValueKey(own),
              ownMessage: own,
              bubble: true,
              content: List.filled(6, '![photo]($first)').join('\n'),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final largeStack = find.byKey(const ValueKey('message-media-stack'));
        for (final element
            in find
                .descendant(
                  of: largeStack,
                  matching: find.byType(ClipRSuperellipse),
                )
                .evaluate()) {
          final box = element.renderObject! as RenderBox;
          final edge =
              box.localToGlobal(Offset(box.size.width, 0)) -
              box.localToGlobal(Offset.zero);
          expect((edge.dy / edge.dx).abs(), lessThan(own ? .12 : .2));
        }
        Finder rearClip(int index) => find.descendant(
          of: find.byKey(ValueKey('stack-card:$index')),
          matching: find.byType(ClipRSuperellipse),
        );
        final rearBefore = {
          for (final i in [2, 3]) i: tester.getRect(rearClip(i)),
        };
        final rearCenters =
            rearBefore.values.map((rect) => rect.center.dx).toList()..sort();
        expect(
          rearCenters.last - rearCenters.first,
          lessThan(1),
          reason: 'Large collections use compact backing tiers',
        );
        List<Key?> rearPaintOrder() => tester
            .widget<Stack>(
              find.byKey(const ValueKey('message-media-card-deck')),
            )
            .children
            .map((child) => child.key)
            .where((key) => [2, 3].any((i) => key == ValueKey('stack-card:$i')))
            .toList();
        final stableRearOrder = rearPaintOrder();
        final deckDrag = await tester.startGesture(
          tester.getCenter(largeStack),
        );
        await deckDrag.moveBy(const Offset(-20, 0));
        void checkDeckBounds() {
          final stage = tester.getRect(largeStack);
          for (final element
              in find
                  .descendant(
                    of: largeStack,
                    matching: find.byType(ClipRSuperellipse),
                  )
                  .evaluate()) {
            final box = element.renderObject! as RenderBox;
            for (final corner in [
              Offset.zero,
              Offset(box.size.width, 0),
              Offset(0, box.size.height),
              Offset(box.size.width, box.size.height),
            ]) {
              expect(stage.contains(box.localToGlobal(corner)), isTrue);
            }
          }
        }

        for (var step = 0; step < 50; step++) {
          await deckDrag.moveBy(const Offset(-1.4, 0));
          await tester.pump();
          checkDeckBounds();
          expect(
            rearPaintOrder(),
            stableRearOrder,
            reason: 'Rear cards must not swap layers during a handoff',
          );
        }
        expect(
          find.descendant(
            of: largeStack,
            matching: find.byType(ClipRSuperellipse),
          ),
          findsNWidgets(4),
        );
        for (final entry in rearBefore.entries) {
          final moved = tester.getRect(rearClip(entry.key));
          expect(
            moved.width,
            lessThan(entry.value.width * .9),
            reason: 'Rear card ${entry.key} must turn with the deck',
          );
          expect(
            (moved.center.dx - entry.value.center.dx).abs(),
            greaterThan(8),
            reason: 'Rear card ${entry.key} must travel with the deck',
          );
        }
        for (var step = 0; step < 49; step++) {
          await deckDrag.moveBy(const Offset(-1.4, 0));
          await tester.pump();
          checkDeckBounds();
          expect(
            rearPaintOrder(),
            stableRearOrder,
            reason: 'Rear cards must not swap layers during a handoff',
          );
        }
        // The spring can render arbitrarily close to the next integer. The
        // arriving photo must remain in front until and after that boundary.
        await deckDrag.moveBy(const Offset(-1.393, 0));
        await tester.pump();
        final nearSettledDeck = tester.widget<Stack>(
          find.byKey(const ValueKey('message-media-card-deck')),
        );
        final photoKeys = nearSettledDeck.children
            .map((child) => child.key)
            .where(
              (key) =>
                  key is ValueKey<String> &&
                  key.value.startsWith('stack-card:'),
            );
        expect(
          photoKeys.last,
          const ValueKey('stack-card:1'),
          reason: 'Do not flash the departing image just before settling',
        );
        double shadowStrength() => tester
            .widgetList<DecoratedBox>(
              find.descendant(
                of: largeStack,
                matching: find.byType(DecoratedBox),
              ),
            )
            .map((box) => box.decoration)
            .whereType<ShapeDecoration>()
            .expand((decoration) => decoration.shadows ?? <BoxShadow>[])
            .fold(0.0, (sum, shadow) => sum + shadow.color.a);
        final shadowBefore = shadowStrength();
        expect(shadowBefore, closeTo(3 * 48 / 255, .001));
        final outgoingBox = tester.renderObject<RenderBox>(rearClip(0));
        final coveringBox = tester.renderObject<RenderBox>(rearClip(3));
        for (final corner in [
          Offset.zero,
          Offset(outgoingBox.size.width, outgoingBox.size.height),
        ]) {
          expect(
            (outgoingBox.localToGlobal(corner) -
                    coveringBox.localToGlobal(corner))
                .distance,
            lessThan(.01),
            reason: 'Outgoing image is fully covered before removal',
          );
        }
        await deckDrag.up();
        await tester.pumpAndSettle();
        expect(
          shadowStrength(),
          closeTo(shadowBefore, .001),
          reason: 'The rear shadow must survive the integer handoff',
        );
        expect(find.byKey(const ValueKey('stack-card:0')), findsNothing);
        expect(find.byKey(const ValueKey('stack-card:3')), findsOneWidget);
        // Start each next swipe while the previous spring is still running.
        for (final dx in [-90.0, -90.0, 90.0, -90.0, 90.0, 90.0]) {
          await tester.fling(largeStack, Offset(dx, 0), 2000);
          for (var frame = 0; frame < 3; frame++) {
            await tester.pump(const Duration(milliseconds: 16));
            final value = tester
                .widget<Semantics>(
                  find.byKey(const ValueKey('message-media-stack-semantics')),
                )
                .properties
                .value!;
            final selected = int.parse(value.split(' ')[1]) - 1;
            final keys = tester
                .widget<Stack>(
                  find.byKey(const ValueKey('message-media-card-deck')),
                )
                .children
                .map((child) => child.key)
                .where(
                  (key) =>
                      key is ValueKey<String> &&
                      key.value.startsWith('stack-card:'),
                );
            expect(keys.last, ValueKey('stack-card:$selected'));
            expect(tester.takeException(), isNull);
          }
        }
        await tester.pumpAndSettle();
      }
      await tester.pumpWidget(
        _testable(
          const MessageContent(
            bubble: true,
            content: '![one]($first)\n![duplicate]($first)',
          ),
          disableAnimations: true,
        ),
      );
      await tester.pumpAndSettle();
      final spoken = find.byKey(
        const ValueKey('message-media-stack-semantics'),
      );
      final node = tester.getSemantics(spoken);
      node.owner!.performAction(node.id, SemanticsAction.decrease);
      await tester.pump();
      expect(
        tester
            .widget<Semantics>(
              find.byKey(const ValueKey('message-media-stack-semantics')),
            )
            .properties
            .value,
        'Photo 2 of 2',
      );
      expect(tester.binding.hasScheduledFrame, isFalse);
      expect(tester.takeException(), isNull);
      semantics.dispose();
    },
  );
  testWidgets('stack tuner adjusts slots independently and resets', (
    tester,
  ) async {
    String? copiedSettings;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copiedSettings = (call.arguments as Map)['text'] as String;
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    _setSurfaceSize(tester, const Size(430, 932));
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      _testable(
        MessageContent(
          bubble: true,
          ownMessage: true,
          content: List.filled(
            6,
            '![photo](https://example.com/tuning.png)',
          ).join('\n'),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Tune stack'));
    await tester.pumpAndSettle();
    final sheet = find.byKey(const ValueKey('stack-tuning-sheet'));
    Finder clip(int i) => find.descendant(
      of: find.descendant(
        of: sheet,
        matching: find.byKey(ValueKey('stack-card:$i')),
      ),
      matching: find.byType(ClipRSuperellipse),
    );
    final first = tester.getRect(clip(0));
    final second = tester.getRect(clip(1));
    final third = tester.getRect(clip(2));
    await tester.tap(find.text('Card 2'));
    await tester.pumpAndSettle();
    final sliders = find.descendant(of: sheet, matching: find.byType(Slider));
    await tester.drag(sliders.at(0), const Offset(45, 0));
    await tester.pumpAndSettle();
    expect(
      tester.getRect(clip(1)).center.dx,
      greaterThan(second.center.dx + 1),
    );
    expect(tester.getRect(clip(0)), first);
    expect(tester.getRect(clip(2)), third);
    await tester.drag(sliders.at(1), const Offset(35, 0));
    await tester.pumpAndSettle();
    expect(
      tester.getRect(clip(1)).center.dy,
      greaterThan(second.center.dy + 1),
    );
    await tester.drag(sliders.at(2), const Offset(35, 0));
    await tester.pumpAndSettle();
    expect(tester.widget<Slider>(sliders.at(2)).value, greaterThan(0));
    await tester.tap(find.text('Copy settings'));
    await tester.pumpAndSettle();
    expect(copiedSettings, isNotNull);
    final exported = jsonDecode(copiedSettings!) as List;
    expect(exported[0]['x'], 0);
    expect(exported[1]['x'], greaterThan(0));
    expect(exported[2]['x'], 0);
    await tester.tap(find.text('Reset all'));
    await tester.pumpAndSettle();
    expect(tester.getRect(clip(1)), second);
    expect(tester.widget<Slider>(sliders.at(0)).value, 0);
    expect(tester.widget<Slider>(sliders.at(1)).value, 0);
    expect(tester.widget<Slider>(sliders.at(2)).value, 0);
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    expect(sheet, findsNothing);
    expect(tester.takeException(), isNull);
  }, skip: !const bool.fromEnvironment('BUZZ_PHOTO_STACK_TUNING'));
}
