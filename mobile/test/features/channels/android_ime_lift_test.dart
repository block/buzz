import 'package:buzz/features/channels/android_ime_lift.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('lifts only the composer on Android', (tester) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      await tester.pumpWidget(_testLayout());

      expect(
        tester.getBottomLeft(find.byKey(const ValueKey('composer'))).dy,
        280,
      );
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });

  testWidgets('leaves the iOS composer on the scaffold resize path', (
    tester,
  ) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      await tester.pumpWidget(_testLayout());

      expect(
        tester.getBottomLeft(find.byKey(const ValueKey('composer'))).dy,
        400,
      );
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });

  testWidgets('does not double-count Android navigation safe area', (
    tester,
  ) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      await tester.pumpWidget(
        _testLayout(viewPadding: 24, reservesViewPadding: true),
      );

      final keyboardTop = 400 - 120;
      final composerBottom = tester
          .getBottomLeft(find.byKey(const ValueKey('composer')))
          .dy;
      expect(keyboardTop - composerBottom, Grid.xxs);
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });
}

Widget _testLayout({double viewPadding = 0, bool reservesViewPadding = false}) {
  return MediaQuery(
    data: MediaQueryData(
      viewInsets: const EdgeInsets.only(bottom: 120),
      viewPadding: EdgeInsets.only(bottom: viewPadding),
    ),
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: 400,
          height: 400,
          child: AndroidImeLift(
            child: Align(
              alignment: Alignment.bottomCenter,
              child: Padding(
                padding: EdgeInsets.only(
                  bottom: reservesViewPadding ? viewPadding + Grid.xxs : 0,
                ),
                child: const SizedBox(
                  key: ValueKey('composer'),
                  width: 100,
                  height: 40,
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );
}
