import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/app_theme.dart';

void main() {
  test('disables Material touch ripples in every app theme', () {
    expect(AppTheme.light().splashFactory, NoSplash.splashFactory);
    expect(AppTheme.dark().splashFactory, NoSplash.splashFactory);
  });

  test('uses Inter and the shared elevated popover treatment', () {
    final theme = AppTheme.light();
    final popupTheme = theme.popupMenuTheme;
    final shape = popupTheme.shape! as RoundedRectangleBorder;
    final side = shape.side;

    expect(popupTheme.textStyle?.fontFamily, 'Inter');
    expect(popupTheme.elevation, 8);
    expect(
      popupTheme.shadowColor,
      theme.colorScheme.shadow.withValues(alpha: 0.18),
    );
    expect(shape.borderRadius, BorderRadius.circular(Radii.popover));
    expect(side.color, Colors.black.withValues(alpha: 0.04));
    expect(side.width, 1);
  });
}
