import 'package:buzz/features/channels/day_divider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final dark in [false, true]) {
    testWidgets('inline calendar shows local day before label, dark=$dark', (
      tester,
    ) async {
      final date = DateTime(2026, 10, 18, 12);
      final timestamp = date.millisecondsSinceEpoch ~/ 1000;
      await tester.pumpWidget(
        MaterialApp(
          theme: dark ? AppTheme.dark() : AppTheme.light(),
          home: Scaffold(
            body: MediaQuery(
              data: const MediaQueryData(textScaler: TextScaler.linear(2)),
              child: SizedBox(
                width: 320,
                child: DayDivider(
                  label: 'October 18, 2026',
                  dayTimestamp: timestamp,
                ),
              ),
            ),
          ),
        ),
      );
      final tile = find.byKey(ValueKey('day-calendar-$timestamp'));
      expect(
        find.descendant(of: tile, matching: find.text('18')),
        findsOneWidget,
      );
      expect(
        tester.getRect(tile).right,
        lessThan(tester.getRect(find.text('October 18, 2026')).left),
      );
      expect(find.byType(Divider), findsNWidgets(2));
      expect(tester.takeException(), isNull);
    });
  }

  for (final label in ['Today', 'October 18, 2026']) {
    testWidgets('date group centered with full-width rules: $label', (
      tester,
    ) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: SizedBox(
              width: 390,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
                child: DayDivider(label: label, dayTimestamp: 1000),
              ),
            ),
          ),
        ),
      );
      final left = tester.getRect(find.byType(Divider).first);
      final right = tester.getRect(find.byType(Divider).last);
      final icon = tester.getRect(
        find.byKey(const ValueKey('day-calendar-1000')),
      );
      final text = tester.getRect(find.text(label));
      expect(left.left, closeTo(Grid.gutter, 0.01));
      expect(right.right, closeTo(390 - Grid.gutter, 0.01));
      expect(left.width, closeTo(right.width, 0.01));
      expect((icon.left + text.right) / 2, closeTo(195, 0.01));
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('fades the in-flow date while that day is sticky', (
    tester,
  ) async {
    final stickyDayTimestamp = ValueNotifier<int?>(null);
    final scrollVisible = ValueNotifier(false);
    final semantics = tester.ensureSemantics();

    addTearDown(scrollVisible.dispose);
    addTearDown(stickyDayTimestamp.dispose);

    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: DayDivider(
              label: 'Today',
              dayTimestamp: 1000,
              stickyDayTimestamp: stickyDayTimestamp,
              scrollVisible: scrollVisible,
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

      expect(opacity().opacity, 1);
      expect(find.bySemanticsLabel('Today'), findsOneWidget);
      scrollVisible.value = true;
      await tester.pump();
      expect(opacity().opacity, 0);
      expect(
        tester
            .widget<ExcludeSemantics>(
              find
                  .ancestor(
                    of: find.byKey(
                      const ValueKey('channel-day-divider-opacity-1000'),
                    ),
                    matching: find.byType(ExcludeSemantics),
                  )
                  .first,
            )
            .excluding,
        isTrue,
      );
      expect(opacity().duration, const Duration(milliseconds: 120));
      expect(opacity().curve, Curves.easeOutCubic);

      scrollVisible.value = false;
      await tester.pumpAndSettle();
      expect(opacity().opacity, 1);
      expect(find.bySemanticsLabel('Today'), findsOneWidget);

      stickyDayTimestamp.value = null;
      await tester.pump();

      expect(opacity().opacity, 1);
    } finally {
      semantics.dispose();
    }
  });
}
