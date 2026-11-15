import 'package:buzz/features/channels/me/solo_working_dots.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  List<double> opacities(WidgetTester tester, [int row = 0]) => tester
      .widgetList<Opacity>(
        find.descendant(
          of: find.byType(SoloWorkingDots).at(row),
          matching: find.byType(Opacity),
        ),
      )
      .map((dot) => dot.opacity)
      .toList();

  Widget dots({bool ticking = true, int rows = 1}) => Directionality(
    textDirection: TextDirection.ltr,
    child: TickerMode(
      enabled: ticking,
      child: Column(
        children: [
          for (var i = 0; i < rows; i++)
            const SoloWorkingDots(color: Color(0xFF000000)),
        ],
      ),
    ),
  );

  testWidgets('rows step together at about 12 fps with no frames between', (
    tester,
  ) async {
    await tester.pumpWidget(dots(rows: 2));
    expect(debugSoloWorkingDotsClockRunning, isTrue);
    final start = opacities(tester);
    expect(tester.binding.hasScheduledFrame, isFalse);

    // Check before pumping: pump draws any frame scheduled in the interval.
    await tester.binding.delayed(const Duration(milliseconds: 80));
    expect(tester.binding.hasScheduledFrame, isFalse);
    await tester.pump();
    expect(opacities(tester), start);

    await tester.binding.delayed(const Duration(milliseconds: 3));
    expect(tester.binding.hasScheduledFrame, isTrue);
    await tester.pump();
    final next = opacities(tester);
    expect(next, isNot(start));
    expect(opacities(tester, 1), next);
    expect(tester.binding.hasScheduledFrame, isFalse);

    await tester.pumpWidget(const SizedBox());
    expect(debugSoloWorkingDotsClockRunning, isFalse);
  });

  testWidgets('the clock stops while TickerMode is off and resumes after', (
    tester,
  ) async {
    await tester.pumpWidget(dots(ticking: false));
    expect(debugSoloWorkingDotsClockRunning, isFalse);
    final start = opacities(tester);
    await tester.pump(const Duration(seconds: 1));
    expect(opacities(tester), start);
    expect(tester.binding.hasScheduledFrame, isFalse);

    await tester.pumpWidget(dots());
    expect(debugSoloWorkingDotsClockRunning, isTrue);
    await tester.pump(const Duration(milliseconds: 83));
    expect(opacities(tester), isNot(start));

    await tester.pumpWidget(dots(ticking: false));
    expect(debugSoloWorkingDotsClockRunning, isFalse);
  });

  testWidgets('the clock stops while the app is in the background', (
    tester,
  ) async {
    await tester.pumpWidget(dots());
    void go(AppLifecycleState state) =>
        tester.binding.handleAppLifecycleStateChanged(state);
    go(AppLifecycleState.inactive);
    expect(debugSoloWorkingDotsClockRunning, isTrue);
    go(AppLifecycleState.hidden);
    expect(debugSoloWorkingDotsClockRunning, isFalse);
    go(AppLifecycleState.inactive);
    go(AppLifecycleState.resumed);
    expect(debugSoloWorkingDotsClockRunning, isTrue);
    await tester.pumpWidget(const SizedBox());
    expect(debugSoloWorkingDotsClockRunning, isFalse);
  });
}
