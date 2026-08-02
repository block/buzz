import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/app_theme.dart';

void main() {
  test('disables Material touch ripples in every app theme', () {
    expect(AppTheme.light().splashFactory, NoSplash.splashFactory);
    expect(AppTheme.dark().splashFactory, NoSplash.splashFactory);
  });
}
