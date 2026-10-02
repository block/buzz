import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('utility containers use the stronger grouped-surface radius', () {
    expect(Radii.container, 22);
    expect(Radii.card, Radii.container);
  });

  test('exchanges utility canvas and container surface roles once', () {
    const pageSurface = Color(0xFFEEEEEE);
    const containerSurface = Color(0xFFFFFFFF);
    final base = AppTheme.light(
      colorScheme: lightColorScheme.copyWith(
        surface: containerSurface,
        surfaceContainerHighest: pageSurface,
      ),
    );

    final utility = utilitySurfaceThemeData(base);

    expect(utility.scaffoldBackgroundColor, pageSurface);
    expect(utility.colorScheme.surface, pageSurface);
    expect(utility.colorScheme.surfaceContainerHighest, containerSurface);
    expect(utility.cardTheme.color, containerSurface);
    expect(
      (utility.cardTheme.shape! as RoundedRectangleBorder).borderRadius,
      BorderRadius.circular(Radii.container),
    );
    for (final border in [
      utility.inputDecorationTheme.border,
      utility.inputDecorationTheme.enabledBorder,
      utility.inputDecorationTheme.focusedBorder,
      utility.inputDecorationTheme.errorBorder,
      utility.inputDecorationTheme.focusedErrorBorder,
    ]) {
      expect(
        (border! as OutlineInputBorder).borderRadius,
        BorderRadius.circular(Radii.container),
      );
    }
    expect(utility.bottomSheetTheme.backgroundColor, pageSurface);
    expect(utilitySurfaceThemeData(utility), same(utility));
  });
}
