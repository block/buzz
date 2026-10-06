import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:buzz/features/pairing/pairing_page/onboarding_wordmark.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'animates texture, pauses in background, and respects reduced motion',
    (tester) async {
      Future<void> show({
        bool reduceMotion = false,
        bool visible = true,
      }) async {
        await tester.pumpWidget(
          MaterialApp(
            home: MediaQuery(
              data: MediaQueryData(disableAnimations: reduceMotion),
              child: TickerMode(
                enabled: visible,
                child: const Center(
                  child: SizedBox(
                    width: 320,
                    height: 320 * 326 / 777,
                    child: RepaintBoundary(
                      key: Key('capture'),
                      child: OnboardingWordmark(),
                    ),
                  ),
                ),
              ),
            ),
          ),
        );
      }

      await show();
      final texture = find.byKey(const Key('pairing-buzz-animated-texture'));
      for (
        var attempt = 0;
        attempt < 100 && texture.evaluate().isEmpty;
        attempt++
      ) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
      expect(
        texture,
        findsOneWidget,
        reason: 'Exercise the real compiled shader',
      );
      expect(find.bySemanticsLabel('Buzz'), findsOneWidget);

      Future<Uint8List> pixels() async {
        return (await tester.runAsync(() async {
          final boundary = tester.renderObject<RenderRepaintBoundary>(
            find.byKey(const Key('capture')),
          );
          final image = await boundary.toImage();
          try {
            final data = await image.toByteData(
              format: ui.ImageByteFormat.rawRgba,
            );
            return Uint8List.fromList(data!.buffer.asUint8List());
          } finally {
            image.dispose();
          }
        }))!;
      }

      final first = await pixels();
      await tester.pump(const Duration(milliseconds: 100));
      expect(await pixels(), isNot(orderedEquals(first)));

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      final paused = await pixels();
      await tester.pump(const Duration(milliseconds: 150));
      expect(await pixels(), orderedEquals(paused));
      expect(tester.binding.hasScheduledFrame, isFalse);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      expect(await pixels(), isNot(orderedEquals(paused)));

      await show(reduceMotion: true);
      await tester.pumpAndSettle();
      expect(texture, findsNothing);
      expect(find.byType(Image), findsOneWidget);
      expect(tester.binding.hasScheduledFrame, isFalse);

      await show(visible: false);
      await tester.pumpAndSettle();
      expect(texture, findsNothing);
      expect(tester.binding.hasScheduledFrame, isFalse);
      await tester.pumpWidget(const SizedBox());
      expect(tester.takeException(), isNull);
    },
  );
}
