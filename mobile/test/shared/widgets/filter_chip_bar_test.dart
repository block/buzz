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
  });
}
