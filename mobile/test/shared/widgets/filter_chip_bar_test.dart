import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/filter_chip_bar.dart';

void main() {
  testWidgets('selected chip resolves the accent (scheme.primary) as fill', (
    tester,
  ) async {
    final accented = applyAccent(darkColorScheme, 0); // Blue accent
    final accent = accented.primary;

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.dark(colorScheme: accented),
        home: Scaffold(
          body: FilterChipBar<int>(
            selected: 0,
            onSelected: (_) {},
            items: const [
              FilterChipItem(id: 0, label: 'Everyone'),
              FilterChipItem(id: 1, label: 'Following'),
            ],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    // The chip's container color comes from ChipThemeData.color resolved for
    // the selected state. Resolve it the same way the chip does and assert
    // it's the accent.
    final chipTheme = ChipTheme.of(
      tester.element(find.widgetWithText(RawChip, 'Everyone')),
    );
    final resolved = chipTheme.color?.resolve({WidgetState.selected});
    expect(resolved, accent);
    final selectedLabel = tester.widget<Text>(find.text('Everyone'));
    final unselectedLabel = tester.widget<Text>(find.text('Following'));
    expect(selectedLabel.style?.fontSize, filterChipTextStyle.fontSize);
    expect(selectedLabel.style?.height, filterChipTextStyle.height);
    expect(selectedLabel.style?.fontFamily, 'Inter');
    expect(selectedLabel.style?.fontWeight, FontWeight.w500);
    expect(unselectedLabel.style?.fontSize, filterChipTextStyle.fontSize);
    expect(unselectedLabel.style?.height, filterChipTextStyle.height);
    expect(unselectedLabel.style?.fontWeight, FontWeight.w400);
    final chip = tester.widget<FilterChip>(
      find.widgetWithText(FilterChip, 'Everyone'),
    );
    final shape = chip.shape! as RoundedRectangleBorder;
    expect(shape.borderRadius, BorderRadius.circular(Radii.lg));
    expect(chip.labelPadding, EdgeInsets.zero);
  });

  testWidgets('search labels are vertically centered in their chips', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: SizedBox(
            width: 390,
            child: FilterChipBar<int>(
              expandItems: true,
              visualDensity: const VisualDensity(horizontal: -2),
              chipVerticalPadding: Grid.xxs,
              barVerticalPadding: Grid.twelve,
              selected: 0,
              onSelected: (_) {},
              items: const [
                FilterChipItem(id: 0, label: 'All'),
                FilterChipItem(id: 1, label: 'Messages'),
                FilterChipItem(id: 2, label: 'Channels'),
                FilterChipItem(id: 3, label: 'People'),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    for (final label in ['All', 'Messages', 'Channels', 'People']) {
      final text = find.text(label);
      final chip = find.ancestor(of: text, matching: find.byType(RawChip));
      expect(chip, findsOneWidget);
      expect(
        tester.getCenter(text).dy,
        closeTo(tester.getCenter(chip).dy, 0.01),
      );
    }
  });

  testWidgets('expanded chips preserve large accessible text scaling', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(2)),
          child: Scaffold(
            body: SizedBox(
              width: 240,
              child: FilterChipBar<int>(
                expandItems: true,
                selected: 0,
                onSelected: (_) {},
                items: const [
                  FilterChipItem(id: 0, label: 'All'),
                  FilterChipItem(id: 1, label: 'Messages'),
                  FilterChipItem(id: 2, label: 'Channels'),
                  FilterChipItem(id: 3, label: 'People'),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(
      find.descendant(
        of: find.byType(FilterChipBar<int>),
        matching: find.byType(FittedBox),
      ),
      findsNothing,
    );
    expect(
      tester.getSize(find.text('Messages')).height,
      greaterThanOrEqualTo(30),
    );
    expect(tester.takeException(), isNull);
  });
}
