import 'package:buzz/features/channels/date_header_scroll_activity.dart';
import 'package:buzz/features/channels/sticky_date_header.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('date follows reader scrolling, expires, and returns immediately', (
    tester,
  ) async {
    final activity = DateHeaderScrollActivity();
    final state = ValueNotifier(
      const StickyDateHeaderState(label: 'Yesterday'),
    );
    addTearDown(activity.dispose);
    addTearDown(state.dispose);
    late BuildContext scrollContext;
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Column(
            children: [
              StickyDateHeader(state: state, scrollVisible: activity),
              Builder(
                builder: (context) {
                  scrollContext = context;
                  return const SizedBox();
                },
              ),
            ],
          ),
        ),
      ),
    );
    final metrics = FixedScrollMetrics(
      minScrollExtent: 0,
      maxScrollExtent: 1000,
      pixels: 100,
      viewportDimension: 500,
      axisDirection: AxisDirection.down,
      devicePixelRatio: 1,
    );
    double opacity() =>
        tester.widget<AnimatedOpacity>(find.byType(AnimatedOpacity)).opacity;
    void update() => activity.onNotification(
      ScrollUpdateNotification(
        metrics: metrics,
        context: scrollContext,
        scrollDelta: 10,
      ),
    );
    void start() => activity.onNotification(
      ScrollStartNotification(
        metrics: metrics,
        context: scrollContext,
        dragDetails: DragStartDetails(),
      ),
    );
    void end() => activity.onNotification(
      ScrollEndNotification(metrics: metrics, context: scrollContext),
    );

    expect(opacity(), 0);
    update(); // Initial positioning and incoming-message autoscroll stay quiet.
    await tester.pump();
    expect(opacity(), 0);
    start();
    await tester.pump();
    expect(opacity(), 1);
    expect(
      tester.widget<AnimatedOpacity>(find.byType(AnimatedOpacity)).duration,
      Duration.zero,
    );
    await tester.pump(const Duration(milliseconds: 800));
    update(); // Momentum refreshes the deadline, even without drag details.
    await tester.pump(const Duration(milliseconds: 800));
    expect(opacity(), 1);
    end();
    await tester.pump(const Duration(milliseconds: 999));
    expect(opacity(), 1);
    await tester.pump(const Duration(milliseconds: 1));
    expect(opacity(), 0);
    expect(
      tester.widget<AnimatedOpacity>(find.byType(AnimatedOpacity)).duration,
      const Duration(milliseconds: 250),
    );
    expect(
      find.text('Yesterday'),
      findsOneWidget,
    ); // Keep the label during fade.
    await tester.pump(const Duration(milliseconds: 125));
    final fade = tester.widget<FadeTransition>(
      find.descendant(
        of: find.byType(AnimatedOpacity),
        matching: find.byType(FadeTransition),
      ),
    );
    expect(fade.opacity.value, closeTo(0.5, 0.05));
    start(); // Interrupt the fade without waiting for it to finish.
    await tester.pump();
    expect(opacity(), 1);
    end();
    await tester.pump(const Duration(seconds: 1));
    await tester.pump(const Duration(milliseconds: 250));
    update();
    await tester.pump();
    expect(opacity(), 0);
  });

  testWidgets('horizontal and nested scrolling do not reveal the date', (
    tester,
  ) async {
    final activity = DateHeaderScrollActivity();
    addTearDown(activity.dispose);
    await tester.pumpWidget(const MaterialApp(home: SizedBox()));
    final context = tester.element(find.byType(SizedBox));
    for (final direction in [AxisDirection.right, AxisDirection.down]) {
      final notification = ScrollStartNotification(
        metrics: FixedScrollMetrics(
          minScrollExtent: 0,
          maxScrollExtent: 1000,
          pixels: 100,
          viewportDimension: 500,
          axisDirection: direction,
          devicePixelRatio: 1,
        ),
        context: context,
        dragDetails: DragStartDetails(),
      );
      if (direction == AxisDirection.down) {
        activity.onNotification(
          ScrollUpdateNotification(
            metrics: notification.metrics,
            context: context,
            depth: 1,
            dragDetails: DragUpdateDetails(globalPosition: Offset.zero),
            scrollDelta: 10,
          ),
        );
      } else {
        activity.onNotification(notification);
      }
      expect(activity.value, false);
    }
  });

  testWidgets('disposing cancels an outstanding idle deadline', (tester) async {
    final activity = DateHeaderScrollActivity();
    await tester.pumpWidget(const MaterialApp(home: SizedBox()));
    activity.onNotification(
      ScrollStartNotification(
        metrics: FixedScrollMetrics(
          minScrollExtent: 0,
          maxScrollExtent: 1000,
          pixels: 100,
          viewportDimension: 500,
          axisDirection: AxisDirection.down,
          devicePixelRatio: 1,
        ),
        context: tester.element(find.byType(SizedBox)),
        dragDetails: DragStartDetails(),
      ),
    );
    activity.dispose();
    await tester.pump(const Duration(seconds: 2));
    expect(tester.takeException(), isNull);
  });
}
