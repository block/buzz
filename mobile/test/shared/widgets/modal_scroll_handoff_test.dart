import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/modal_presentation.dart';
import 'package:buzz/shared/widgets/modal_scroll_handoff.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _bodyKey = ValueKey('handoff-body');
const _horizontalKey = ValueKey('horizontal');
const _nestedKey = ValueKey('nested');

// Only the surrounding layout and platform scroll policy vary. The controller,
// position, drag, modal animation, chrome and navigator are all production code.
class _Harness {
  final navigator = GlobalKey<NavigatorState>();
  final height = ValueNotifier(360.0);
  final visible = ValueNotifier(true);
  final bouncing = ValueNotifier(false);

  AnimationController animation(WidgetTester tester) =>
      tester.widget<BottomSheet>(find.byType(BottomSheet)).animationController!;

  ScrollPosition position(WidgetTester tester) => tester
      .state<ScrollableState>(
        find
            .descendant(
              of: find.byType(ModalScrollHandoff),
              matching: find.byType(Scrollable),
            )
            .first,
      )
      .position;

  double sheetHeight(WidgetTester tester) =>
      tester.getSize(find.byType(BottomSheet)).height;

  Future<void> open(
    WidgetTester tester, {
    bool short = false,
    bool reducedMotion = false,
    bool enableDrag = true,
    bool settle = true,
    Widget? child,
  }) async {
    addTearDown(height.dispose);
    addTearDown(visible.dispose);
    addTearDown(bouncing.dispose);
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          navigatorKey: navigator,
          theme: AppTheme.light(),
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(disableAnimations: reducedMotion),
            child: child!,
          ),
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () => showBuzzModalBottomSheet<void>(
                  context: context,
                  title: 'Handoff',
                  showDragHandle: true,
                  enableDrag: enableDrag,
                  isScrollControlled: true,
                  builder: (_) => ValueListenableBuilder(
                    valueListenable: height,
                    builder: (context, height, _) => SizedBox(
                      height: height,
                      child: ValueListenableBuilder(
                        valueListenable: visible,
                        builder: (context, visible, _) => visible
                            ? ValueListenableBuilder(
                                valueListenable: bouncing,
                                builder: (context, bouncing, _) =>
                                    ScrollConfiguration(
                                      behavior: _ScrollPolicy(bouncing),
                                      child: ModalScrollHandoff(
                                        child:
                                            child ??
                                            SizedBox(
                                              key: _bodyKey,
                                              height: short ? 40 : 1400,
                                              width: double.infinity,
                                              child: const Text('Body content'),
                                            ),
                                      ),
                                    ),
                              )
                            : const Text('Body removed'),
                      ),
                    ),
                  ),
                ),
                child: const Text('Open'),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open'));
    await tester.pump();
    if (settle) await tester.pumpAndSettle();
  }

  Future<void> close(WidgetTester tester) async {
    if (navigator.currentState!.canPop()) navigator.currentState!.pop();
    await tester.pumpAndSettle();
  }
}

class _ScrollPolicy extends MaterialScrollBehavior {
  const _ScrollPolicy(this.bouncing);
  final bool bouncing;

  @override
  bool shouldNotify(covariant _ScrollPolicy oldDelegate) =>
      bouncing != oldDelegate.bouncing;

  @override
  ScrollPhysics getScrollPhysics(BuildContext context) =>
      bouncing ? const BouncingScrollPhysics() : const ClampingScrollPhysics();
}

Drag _start(ScrollPosition position) =>
    position.drag(DragStartDetails(kind: PointerDeviceKind.touch), () {});

void _move(Drag drag, double dy) => drag.update(
  DragUpdateDetails(
    globalPosition: Offset.zero,
    delta: Offset(0, dy),
    primaryDelta: dy,
  ),
);

void _end(Drag drag, double velocity) => drag.end(
  DragEndDetails(
    velocity: Velocity(pixelsPerSecond: Offset(0, velocity)),
    primaryVelocity: velocity,
  ),
);

void main() {
  testWidgets('one drag consumes content, moves route, reopens, then scrolls', (
    tester,
  ) async {
    final h = _Harness();
    await h.open(tester);
    final position = h.position(tester);
    final animation = h.animation(tester);
    final sheetHeight = h.sheetHeight(tester);
    final top = tester.getTopLeft(find.byType(BottomSheet)).dy;
    position.jumpTo(80);
    final drag = _start(position);
    _move(drag, 50);
    expect(position.pixels, 30);
    expect(animation.value, 1);
    _move(drag, 70);
    expect(position.pixels, 0);
    expect(animation.value, closeTo(1 - 40 / sheetHeight, 1e-9));
    await tester.pump();
    expect(
      tester.getTopLeft(find.byType(BottomSheet)).dy,
      closeTo(top + 40, 0.01),
    );
    _move(drag, -65);
    expect(animation.value, 1);
    expect(position.pixels, closeTo(25, 1e-9));
    _end(drag, 0);
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsOneWidget);
    expect(position.pixels, closeTo(25, 1e-9));
    await h.close(tester);
  });

  testWidgets('raw release velocity and progress use strict stock boundaries', (
    tester,
  ) async {
    // Cross both sides of each threshold. At exactly -700, distance still
    // closes; only a strictly faster upward release vetoes that close.
    for (final progress in [0.499, 0.5, 0.501, 1.0]) {
      for (final velocity in [
        -700.01,
        -700.0,
        -699.99,
        0.0,
        699.99,
        700.0,
        700.01,
      ]) {
        final h = _Harness();
        await h.open(tester);
        final position = h.position(tester);
        final drag = _start(position);
        _move(drag, (1 - progress) * h.sheetHeight(tester));
        expect(h.animation(tester).value, closeTo(progress, 1e-9));
        _end(drag, velocity);
        await tester.pumpAndSettle();
        final closes =
            progress < 1 &&
            (velocity > 700 || (velocity >= -700 && progress < 0.5));
        expect(
          find.text('Handoff'),
          closes ? findsNothing : findsOneWidget,
          reason: 'progress=$progress, raw dy velocity=$velocity',
        );
        if (!closes) {
          expect(h.animation(tester).value, 1);
          // No residual content fling from a sheet-owned release.
          if (progress < 1) expect(position.pixels, 0);
        }
        await h.close(tester);
      }
    }
    // Scrolled content does not change the policy. A gesture that began
    // mid-content, or scrolled away from the top and back in the same touch,
    // releases exactly as one that began at the top: by a little or a lot.
    for (final sameTouch in [false, true]) {
      for (final offset in [6.0, 30.0, 300.0]) {
        for (final (progress, velocity, closes) in [
          (0.9, 2000.0, true),
          (0.501, 700.01, true),
          (0.501, 700.0, false),
          (0.499, 0.0, true),
          (0.499, -700.01, false),
          (1.0, 2000.0, false),
        ]) {
          final h = _Harness();
          await h.open(tester);
          final position = h.position(tester);
          if (!sameTouch) position.jumpTo(offset);
          final drag = _start(position);
          if (sameTouch) _move(drag, -offset);
          expect(position.pixels, offset);
          _move(drag, offset + (1 - progress) * h.sheetHeight(tester));
          expect(position.pixels, 0);
          expect(h.animation(tester).value, closeTo(progress, 1e-9));
          _end(drag, velocity);
          await tester.pumpAndSettle();
          expect(
            find.text('Handoff'),
            closes ? findsNothing : findsOneWidget,
            reason:
                '${sameTouch ? 'same-touch scroll' : 'mid-content start'}, '
                'offset=$offset, progress=$progress, velocity=$velocity',
          );
          if (!closes) {
            expect(h.animation(tester).value, 1);
            if (progress < 1) expect(position.pixels, 0);
          }
          await h.close(tester);
        }
      }
    }
  });

  testWidgets('a downward flick closes through trailing bounce', (
    tester,
  ) async {
    // A short menu under bouncing physics overscrolls on an upward drag
    // although nothing scrolled: in the same touch, and in a new touch during
    // the spring-back. A long menu can be overscrolled past its last action.
    // One update repays the bounce and any content, then moves the sheet, and
    // the flick closes in every case.
    for (final scenario in [
      'short, same-touch nudge',
      'short, spring-back',
      'long, trailing overscroll',
    ]) {
      final h = _Harness();
      await h.open(tester, short: scenario.startsWith('short'));
      h.bouncing.value = true;
      await tester.pumpAndSettle();
      final position = h.position(tester);
      var drag = _start(position);
      switch (scenario) {
        case 'short, same-touch nudge':
          _move(drag, -80);
        case 'short, spring-back':
          _move(drag, -80);
          _end(drag, 0);
          await tester.pump();
          await tester.pump(const Duration(milliseconds: 16));
          drag = _start(position);
        case 'long, trailing overscroll':
          drag.cancel();
          position.jumpTo(position.maxScrollExtent + 40);
          drag = _start(position);
      }
      expect(
        position.pixels - position.maxScrollExtent,
        greaterThan(1),
        reason: scenario,
      );
      _move(drag, position.pixels + 60);
      expect(position.pixels, 0, reason: scenario);
      expect(
        h.animation(tester).value,
        inExclusiveRange(0.5, 1),
        reason: scenario,
      );
      _end(drag, 700.01);
      await tester.pumpAndSettle();
      expect(find.text('Handoff'), findsNothing, reason: scenario);
      await h.close(tester);
    }
  });

  testWidgets('a displaced release cannot zero a later gesture ballistic', (
    tester,
  ) async {
    final h = _Harness();
    await h.open(tester);
    final position = h.position(tester) as ScrollPositionWithSingleContext;
    final displaced = _start(position);
    _move(displaced, 60);
    // Interrupted by an activity that never goes ballistic for this gesture.
    position.beginActivity(IdleScrollActivity(position));
    await tester.pumpAndSettle();
    expect(h.animation(tester).value, 1);
    final scroll = _start(position);
    _move(scroll, -100);
    expect(position.pixels, 100);
    position.goBallistic(1500);
    await tester.pumpAndSettle();
    expect(position.pixels, greaterThan(100));
    await h.close(tester);
  });

  // Pointer events with explicit timestamps, through the real recognizers. A
  // zero velocity rests before lifting, so no fling is reported.
  Future<void> release(
    WidgetTester tester,
    Offset from,
    double dy,
    double velocity,
  ) async {
    const steps = 10;
    final micros = velocity == 0 ? 80000 : (dy / velocity * 1e6).round();
    final gesture = await tester.startGesture(from);
    for (var i = 1; i <= steps; i++) {
      await gesture.moveBy(
        Offset(0, dy / steps),
        timeStamp: Duration(microseconds: micros * i ~/ steps),
      );
    }
    if (velocity == 0) await tester.pump(const Duration(milliseconds: 200));
    await gesture.up(timeStamp: Duration(microseconds: micros));
    await tester.pumpAndSettle();
  }

  for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
    testWidgets(
      'body releases from the top match the stock sheet drag: $platform',
      (tester) async {
        debugDefaultTargetPlatformOverride = platform;
        try {
          // Both sides of the stock 700 px/s and 0.5 progress thresholds, with
          // room for touch slop. A Flutter upgrade that moves either threshold
          // changes the stock outcome and fails here.
          for (final (fraction, velocity, closes) in [
            (0.2, 0.0, false),
            (0.4, 0.0, false),
            (0.6, 0.0, true),
            (0.25, 400.0, false),
            (0.25, 1000.0, true),
          ]) {
            for (final stock in [true, false]) {
              final h = _Harness();
              await h.open(tester);
              final target = stock
                  ? tester.getCenter(
                      find.byKey(const ValueKey('buzz-sheet-drag-handle')),
                    )
                  : tester.getRect(find.byType(ModalScrollHandoff)).topCenter +
                        const Offset(0, 20);
              await release(
                tester,
                target,
                fraction * h.sheetHeight(tester),
                velocity,
              );
              expect(
                find.text('Handoff'),
                closes ? findsNothing : findsOneWidget,
                reason:
                    '${stock ? 'stock handle' : 'body'}, '
                    'fraction=$fraction, velocity=$velocity',
              );
              await h.close(tester);
            }
          }
        } finally {
          debugDefaultTargetPlatformOverride = null;
        }
      },
    );

    // The hand-tested gesture: scroll the list up, slightly or a long way,
    // then swipe down hard so the finger carries content to the top and pulls
    // the sheet. A new touch after lifting, or the same touch that scrolled.
    for (final scenario in [
      'from the top',
      'new touch after a slight scroll',
      'same touch after a slight scroll',
      'same touch after a long scroll',
      'slow return across the top',
    ]) {
      testWidgets('pointer swipe down, $scenario: $platform', (tester) async {
        debugDefaultTargetPlatformOverride = platform;
        try {
          final h = _Harness();
          await h.open(tester);
          h.bouncing.value = platform == TargetPlatform.iOS;
          await tester.pumpAndSettle();
          final from =
              tester.getRect(find.byType(ModalScrollHandoff)).topCenter +
              const Offset(0, 20);
          final long = scenario.contains('long');
          final sameTouch = scenario.startsWith('same touch');
          TestGesture? touch;
          var elapsed = 0;
          if (scenario != 'from the top') {
            // Scroll up slowly and rest, so the scroll itself carries no fling.
            touch = await tester.startGesture(from + const Offset(0, 200));
            for (var i = 1; i <= 6; i++) {
              elapsed = 40000 * i;
              await touch.moveBy(
                Offset(0, long ? -40 : -8),
                timeStamp: Duration(microseconds: elapsed),
              );
            }
            await tester.pump(const Duration(milliseconds: 200));
            elapsed += 200000;
            if (!sameTouch) {
              await touch.up(timeStamp: Duration(microseconds: elapsed));
              await tester.pumpAndSettle();
              touch = null;
            }
            final scrolled = h.position(tester).pixels;
            expect(scrolled, long ? greaterThan(150) : inExclusiveRange(5, 50));
          }
          final scrolled = h.position(tester).pixels;
          if (touch == null) {
            await release(
              tester,
              from,
              scrolled + 80,
              scenario.startsWith('slow') ? 0 : 2000,
            );
          } else {
            const steps = 10;
            final back = scrolled + 80;
            final micros = (back / 2000 * 1e6).round();
            for (var i = 1; i <= steps; i++) {
              await touch.moveBy(
                Offset(0, back / steps),
                timeStamp: Duration(
                  microseconds: elapsed + micros * i ~/ steps,
                ),
              );
            }
            await tester.pump();
            expect(h.position(tester).pixels, 0);
            expect(h.animation(tester).value, inExclusiveRange(0.5, 1));
            await touch.up(timeStamp: Duration(microseconds: elapsed + micros));
            await tester.pumpAndSettle();
          }
          if (scenario.startsWith('slow')) {
            // The finger displaced the sheet less than half way and rested:
            // neither rule closes, and the menu settles open at the top.
            expect(find.text('Handoff'), findsOneWidget);
            expect(h.animation(tester).value, 1);
            expect(h.position(tester).pixels, 0);
            await h.close(tester);
          } else {
            expect(find.text('Handoff'), findsNothing);
          }
        } finally {
          debugDefaultTargetPlatformOverride = null;
        }
      });
    }
  }

  testWidgets('interruptions restore deeply displaced sheets without closing', (
    tester,
  ) async {
    for (final interruption in [
      'cancel',
      'ballistic',
      'activity',
      'jump',
      'physics',
      'detach',
    ]) {
      final h = _Harness();
      await h.open(tester);
      final position = h.position(tester);
      final drag = _start(position);
      _move(drag, h.sheetHeight(tester) * 0.7);
      expect(h.animation(tester).value, closeTo(0.3, 1e-9));
      switch (interruption) {
        case 'cancel':
          drag.cancel();
        case 'ballistic':
          (position as ScrollPositionWithSingleContext).goBallistic(0);
        case 'activity':
          (position as ScrollPositionWithSingleContext).beginActivity(
            IdleScrollActivity(position),
          );
        case 'jump':
          position.jumpTo(0);
        case 'physics':
          h.bouncing.value = true;
          await tester.pump();
          expect(h.position(tester), isNot(same(position)));
          expect(
            h.position(tester).physics.parent,
            isA<BouncingScrollPhysics>(),
          );
        case 'detach':
          h.visible.value = false;
          await tester.pump();
      }
      await tester.pumpAndSettle();
      expect(find.text('Handoff'), findsOneWidget, reason: interruption);
      expect(h.animation(tester).value, 1, reason: interruption);
      if (interruption == 'detach') {
        h.visible.value = true;
        await tester.pumpAndSettle();
      }
      // A fresh gesture must still own its own terminal event after interruption.
      final next = _start(h.position(tester));
      _move(next, h.sheetHeight(tester) * 0.2);
      _end(next, 701);
      await tester.pumpAndSettle();
      expect(find.text('Handoff'), findsNothing, reason: interruption);
    }
  });

  testWidgets('a live pointer cannot release a detached body', (tester) async {
    final h = _Harness();
    await h.open(tester);
    final gesture = await tester.startGesture(
      tester.getBottomLeft(find.byType(BottomSheet)) + const Offset(200, -40),
    );
    await gesture.moveBy(const Offset(0, 24));
    await gesture.moveBy(Offset(0, h.sheetHeight(tester) * 0.7));
    await tester.pump();
    expect(h.animation(tester).value, lessThan(0.5));
    h.visible.value = false;
    await tester.pumpAndSettle();
    expect(h.animation(tester).value, 1);
    h.visible.value = true;
    await tester.pumpAndSettle();
    await gesture.moveBy(const Offset(0, 100));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsOneWidget);
    expect(h.animation(tester).value, 1);
    final next = _start(h.position(tester));
    _move(next, 60);
    _end(next, 701);
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsNothing);
  });

  testWidgets('a system-cancelled pointer restores a deeply displaced sheet', (
    tester,
  ) async {
    final h = _Harness();
    await h.open(tester);
    final from =
        tester.getRect(find.byType(ModalScrollHandoff)).topCenter +
        const Offset(0, 20);
    // Flutter ends an accepted drag on PointerCancelEvent, it does not cancel.
    final cancelled = await tester.startGesture(from);
    await cancelled.moveBy(const Offset(0, 24));
    await cancelled.moveBy(Offset(0, h.sheetHeight(tester) * 0.7));
    await tester.pump();
    expect(h.animation(tester).value, lessThan(0.5));
    await cancelled.cancel();
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsOneWidget);
    expect(h.animation(tester).value, 1);
    // The cancellation does not outlive its pointer: the same pull, lifted,
    // closes on progress.
    final lifted = await tester.startGesture(from);
    await lifted.moveBy(const Offset(0, 24));
    await lifted.moveBy(Offset(0, h.sheetHeight(tester) * 0.7));
    await tester.pump();
    await lifted.up();
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsNothing);
  });

  testWidgets('the sheet has one drag owner across body and handle', (
    tester,
  ) async {
    final h = _Harness();
    await h.open(tester);
    final height = h.sheetHeight(tester);
    // A shallow body pull, then a second finger pulls the handle past half.
    final body = await tester.startGesture(
      tester.getRect(find.byType(ModalScrollHandoff)).topCenter +
          const Offset(0, 20),
    );
    await body.moveBy(const Offset(0, 24));
    await body.moveBy(const Offset(0, 40));
    final handle = await tester.startGesture(
      tester.getCenter(find.byKey(const ValueKey('buzz-sheet-drag-handle'))),
    );
    await handle.moveBy(const Offset(0, 24));
    await handle.moveBy(Offset(0, height * 0.6));
    await tester.pump();
    final displaced = h.animation(tester).value;
    expect(displaced, lessThan(0.5));
    // The handle owns the sheet: the body can no longer move or settle it.
    await body.moveBy(const Offset(0, 30));
    await body.up();
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsOneWidget);
    expect(h.animation(tester).value, displaced);
    await handle.moveBy(Offset(0, -height * 0.5));
    await handle.up();
    await tester.pumpAndSettle();
    expect(h.animation(tester).value, 1);
    // Ownership ends with the gesture: the body's next pull closes.
    final next = _start(h.position(tester));
    _move(next, 60);
    _end(next, 701);
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsNothing);
  });

  testWidgets('carried scroll momentum cannot promote a raw sheet release', (
    tester,
  ) async {
    final h = _Harness();
    await h.open(tester);
    h.bouncing.value = true;
    await tester.pumpAndSettle();
    final position = h.position(tester) as ScrollPositionWithSingleContext;
    position.jumpTo(200);
    position.goBallistic(-1000);
    // Capture an actual bouncing ballistic's velocity just as a pointer down
    // does, retaining enough momentum to take a raw 699 release above 700.
    position.hold(() {});
    final carried = position.physics.carriedMomentum(-1000);
    expect(carried.abs(), greaterThan(700 - 699));
    expect(699, greaterThan(carried.abs() * 0.5));
    final drag = _start(position);
    _move(drag, 260);
    expect(position.pixels, 0);
    expect(h.animation(tester).value, greaterThan(0.5));
    expect(h.animation(tester).value, lessThan(1));
    _end(drag, 699);
    await tester.pumpAndSettle();
    expect(find.text('Handoff'), findsOneWidget);
    expect(h.animation(tester).value, 1);
    expect(position.pixels, 0);
    await h.close(tester);
  });

  testWidgets(
    'short content remains draggable; reduced motion settles immediately',
    (tester) async {
      for (final reduced in [false, true]) {
        final h = _Harness();
        await h.open(tester, short: true, reducedMotion: reduced);
        expect(h.position(tester).maxScrollExtent, 0);
        final animation = h.animation(tester);
        final cancel = _start(h.position(tester));
        _move(cancel, h.sheetHeight(tester) * 0.7);
        expect(animation.value, closeTo(0.3, 1e-9));
        cancel.cancel();
        if (reduced) {
          expect(animation.value, 1);
          expect(animation.isAnimating, isFalse);
        }
        await tester.pumpAndSettle();
        final dismiss = _start(h.position(tester));
        _move(dismiss, h.sheetHeight(tester) * 0.7);
        _end(dismiss, 0);
        if (reduced) {
          expect(animation.value, 0);
          expect(animation.isAnimating, isFalse);
        }
        await tester.pumpAndSettle();
        expect(find.text('Handoff'), findsNothing);
      }
    },
  );

  testWidgets(
    'inherited top and bottom bounce are repaid before route movement',
    (tester) async {
      final h = _Harness();
      await h.open(tester);
      h.bouncing.value = true;
      await tester.pumpAndSettle();
      final position = h.position(tester);
      final animation = h.animation(tester);
      position.jumpTo(-40);
      final drag = _start(position);
      _move(drag, 20);
      expect(position.pixels, lessThan(-40));
      expect(animation.value, 1);
      final debt = -position.pixels;
      _move(drag, -debt / 2);
      expect(position.pixels, closeTo(-debt / 2, 1e-9));
      expect(animation.value, 1);
      _move(drag, -debt / 2 - 30);
      expect(position.pixels, closeTo(30, 1e-9));
      expect(animation.value, 1);
      _move(drag, 50);
      expect(position.pixels, 0);
      expect(animation.value, closeTo(1 - 20 / h.sheetHeight(tester), 1e-9));
      drag.cancel();
      await tester.pumpAndSettle();
      position.jumpTo(position.maxScrollExtent + 40);
      final bottom = _start(position);
      _move(bottom, 20);
      expect(position.pixels, greaterThan(position.maxScrollExtent));
      expect(animation.value, 1);
      bottom.cancel();
      await tester.pumpAndSettle();
      expect(position.pixels, closeTo(position.maxScrollExtent, 0.01));
      await h.close(tester);
    },
  );

  testWidgets(
    'entrance interruption and invalid binding stay ordinary scrolling',
    (tester) async {
      final h = _Harness();
      await h.open(tester, settle: false);
      final animation = h.animation(tester);
      expect(animation.isCompleted, isFalse);
      final entrance = _start(h.position(tester));
      final before = animation.value;
      _move(entrance, 100);
      expect(animation.value, before);
      _end(entrance, 1500);
      await tester.pumpAndSettle();
      expect(animation.value, 1);
      await h.close(tester);

      final disabled = _Harness();
      await disabled.open(tester, enableDrag: false);
      final drag = _start(disabled.position(tester));
      _move(drag, 300);
      _end(drag, 2000);
      await tester.pumpAndSettle();
      expect(disabled.animation(tester).value, 1);
      expect(find.text('Handoff'), findsOneWidget);
      await disabled.close(tester);

      final covered = _Harness();
      await covered.open(tester);
      final coveredPosition = covered.position(tester);
      final coveredAnimation = covered.animation(tester);
      final displaced = _start(coveredPosition);
      _move(displaced, covered.sheetHeight(tester) * 0.7);
      expect(coveredAnimation.value, closeTo(0.3, 1e-9));
      covered.navigator.currentState!.push<void>(
        DialogRoute<void>(
          context: covered.navigator.currentContext!,
          builder: (_) => const AlertDialog(content: Text('Covering route')),
        ),
      );
      await tester.pumpAndSettle();
      displaced.cancel();
      await tester.pumpAndSettle();
      expect(coveredAnimation.value, 1);
      final obscured = _start(coveredPosition);
      _move(obscured, 300);
      _end(obscured, 2000);
      await tester.pumpAndSettle();
      expect(coveredAnimation.value, 1);
      expect(find.text('Covering route'), findsOneWidget);
      covered.navigator.currentState!.pop();
      await tester.pumpAndSettle();
      expect(find.text('Handoff'), findsOneWidget);
      final fresh = _start(coveredPosition);
      _move(fresh, 80);
      expect(coveredAnimation.value, lessThan(1));
      fresh.cancel();
      await tester.pumpAndSettle();
      expect(coveredAnimation.value, 1);
      await covered.close(tester);

      await tester.pumpWidget(
        const ProviderScope(
          child: MaterialApp(
            home: Scaffold(
              body: SizedBox(
                height: 300,
                child: ModalScrollHandoff(child: SizedBox(height: 1400)),
              ),
            ),
          ),
        ),
      );
      final ordinary = disabled.position(tester);
      final outside = _start(ordinary);
      _move(outside, -100);
      expect(ordinary.pixels, greaterThan(0));
      _move(outside, 200);
      outside.cancel();
      await tester.pumpAndSettle();
      expect(find.byType(ModalScrollHandoff), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'layout changes and external pop never turn into a drag release',
    (tester) async {
      final h = _Harness();
      await h.open(tester);
      var drag = _start(h.position(tester));
      _move(drag, h.sheetHeight(tester) * 0.2);
      h.height.value = 240;
      await tester.pumpAndSettle();
      expect(find.text('Handoff'), findsOneWidget);
      drag.cancel();
      await tester.pumpAndSettle();
      expect(h.animation(tester).value, 1);
      // The next drag must use current sheet geometry, not its original height.
      drag = _start(h.position(tester));
      _move(drag, h.sheetHeight(tester) * 0.25);
      expect(h.animation(tester).value, closeTo(0.75, 1e-9));
      final animation = h.animation(tester);
      h.navigator.currentState!.pop();
      expect(animation.status, AnimationStatus.reverse);
      drag.cancel();
      expect(animation.status, AnimationStatus.reverse);
      await tester.pumpAndSettle();
      expect(find.text('Handoff'), findsNothing);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'touch gestures isolate horizontal and deeper vertical scrollables',
    (tester) async {
      final h = _Harness();
      await h.open(
        tester,
        child: Column(
          children: [
            SizedBox(
              height: 90,
              child: ListView(
                key: _horizontalKey,
                scrollDirection: Axis.horizontal,
                children: List.generate(
                  12,
                  (i) => SizedBox(width: 120, child: Text('Card $i')),
                ),
              ),
            ),
            SizedBox(
              height: 130,
              child: ListView(
                key: _nestedKey,
                children: List.generate(
                  20,
                  (i) => SizedBox(height: 40, child: Text('Nested $i')),
                ),
              ),
            ),
            const SizedBox(height: 1200, child: Text('Outer drag area')),
          ],
        ),
      );
      ScrollPosition inner(Key key) => tester
          .state<ScrollableState>(
            find.descendant(
              of: find.byKey(key),
              matching: find.byType(Scrollable),
            ),
          )
          .position;
      await tester.drag(find.byKey(_horizontalKey), const Offset(-180, 0));
      await tester.pumpAndSettle();
      expect(inner(_horizontalKey).pixels, greaterThan(0));
      expect(h.position(tester).pixels, 0);
      expect(h.animation(tester).value, 1);
      await tester.drag(find.byKey(_nestedKey), const Offset(0, -80));
      await tester.pumpAndSettle();
      expect(inner(_nestedKey).pixels, greaterThan(0));
      await tester.drag(find.byKey(_nestedKey), const Offset(0, 250));
      await tester.pumpAndSettle();
      expect(inner(_nestedKey).pixels, 0);
      expect(h.position(tester).pixels, 0);
      expect(h.animation(tester).value, 1);
      // Unlike its descendants, the opted-in body really moves the route by touch.
      final gesture = await tester.startGesture(
        tester.getBottomLeft(find.byType(BottomSheet)) + const Offset(200, -40),
      );
      await gesture.moveBy(const Offset(0, 24)); // Establish the drag arena.
      await gesture.moveBy(const Offset(0, 76));
      await tester.pump();
      expect(h.animation(tester).value, lessThan(1));
      await gesture.cancel();
      await tester.pumpAndSettle();
      expect(h.animation(tester).value, 1);
      await h.close(tester);
    },
  );

  testWidgets(
    'stock handle drag and accessible dismiss survive body cancellation',
    (tester) async {
      final semantics = tester.ensureSemantics();
      // Release before widget-test invariants, including on assertion failure.
      try {
        for (final accessible in [false, true]) {
          final h = _Harness();
          await h.open(tester);
          final drag = _start(h.position(tester));
          _move(drag, 70);
          drag.cancel();
          await tester.pumpAndSettle();
          final handle = find.byKey(const ValueKey('buzz-sheet-drag-handle'));
          expect(handle, findsOneWidget);
          if (accessible) {
            final dismiss = find.bySemanticsLabel('Dismiss').first;
            final node = tester.getSemantics(dismiss);
            expect(node.flagsCollection.isButton, isTrue);
            expect(
              node.getSemanticsData().hasAction(SemanticsAction.tap),
              isTrue,
            );
            tester.binding.performSemanticsAction(
              SemanticsActionEvent(
                type: SemanticsAction.tap,
                viewId: tester.view.viewId,
                nodeId: node.id,
              ),
            );
          } else {
            await tester.drag(handle, Offset(0, h.sheetHeight(tester) * 0.7));
          }
          await tester.pumpAndSettle();
          expect(find.text('Handoff'), findsNothing);
        }
      } finally {
        semantics.dispose();
      }
    },
  );
}
