import 'package:buzz/features/channels/day_divider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('fades the in-flow date while that day is sticky', (
    tester,
  ) async {
    final stickyDayTimestamp = ValueNotifier<int?>(null);
    addTearDown(stickyDayTimestamp.dispose);

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: DayDivider(
            label: 'Today',
            dayTimestamp: 1000,
            stickyDayTimestamp: stickyDayTimestamp,
          ),
        ),
      ),
    );

    AnimatedOpacity opacity() => tester.widget<AnimatedOpacity>(
      find.byKey(const ValueKey('channel-day-divider-opacity-1000')),
    );

    expect(opacity().opacity, 1);

    stickyDayTimestamp.value = 1000;
    await tester.pump();

    expect(opacity().opacity, 0);
    expect(opacity().duration, const Duration(milliseconds: 120));
    expect(opacity().curve, Curves.easeOutCubic);

    stickyDayTimestamp.value = null;
    await tester.pump();

    expect(opacity().opacity, 1);
  });
}
