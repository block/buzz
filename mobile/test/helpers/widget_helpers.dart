import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:buzz/shared/theme/theme.dart';

class WidgetHelpers {
  static Widget testable({
    required Widget child,
    List<Override> overrides = const [],
    bool disableAnimations = false,
  }) {
    return ProviderScope(
      overrides: overrides,
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(disableAnimations: disableAnimations),
            child: Scaffold(body: child),
          ),
        ),
      ),
    );
  }

  /// Pumps until [finder] matches, letting real async work such as an image
  /// decode run between frames. Gives up after about 5 s so the caller's
  /// `expect` still reports a real regression.
  static Future<void> pumpUntilFound(WidgetTester tester, Finder finder) async {
    for (var i = 0; i < 500 && finder.evaluate().isEmpty; i++) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 10)),
      );
      await tester.pump();
    }
  }
}
