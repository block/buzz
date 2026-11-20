import 'dart:ui' as ui;
import 'package:buzz/features/channels/local_message_send_transition.dart';
import 'package:buzz/features/channels/local_send_bubble_surface.dart';
import 'package:buzz/features/channels/local_send_row.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final withOrigin in [false, true]) {
    for (final progress in [0.25, 0.5, 1.0]) {
      testWidgets(
        'flight transforms, hit bounds and semantics agree at $progress origin=$withOrigin',
        (tester) async {
          final semantics = tester.ensureSemantics();
          await tester.pumpWidget(
            MaterialApp(
              home: Center(
                child: SizedBox(
                  width: 300,
                  height: 300,
                  child: Align(
                    alignment: Alignment.topLeft,
                    child: LocalSendProgress(
                      progress: progress,
                      startOffset: 100,
                      origin: withOrigin
                          ? const Rect.fromLTWH(300, 250, 100, 100)
                          : null,
                      child: LocalSendRow(
                        progress: progress,
                        child: LocalSendBubbleSurface(
                          color: Colors.black,
                          shape: const RoundedRectangleBorder(),
                          child: GestureDetector(
                            behavior: HitTestBehavior.opaque,
                            onTap: () {},
                            child: Semantics(
                              label: 'Flying link',
                              child: const SizedBox(width: 100, height: 100),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          );
          final surface = tester.renderObject<RenderBox>(
            find.byType(LocalSendBubbleSurface),
          );
          final row = tester.renderObject<RenderBox>(find.byType(LocalSendRow));
          final target = tester.renderObject<RenderBox>(
            find.byType(GestureDetector),
          );
          final rowOffset = Offset(0, row.size.height - 100);
          final start = withOrigin
              ? const Offset(300, 250) - surface.localToGlobal(Offset.zero)
              : const Offset(0, 100);
          final bubbleOffset = start * (1 - progress);
          final visibleCenter = rowOffset + bubbleOffset + const Offset(50, 50);
          expect(
            target.localToGlobal(Offset.zero),
            row.localToGlobal(rowOffset + bubbleOffset),
          );
          final rowHits = BoxHitTestResult();
          expect(row.hitTest(rowHits, position: visibleCenter), isTrue);
          expect(
            rowHits.path.any((entry) => identical(entry.target, target)),
            isTrue,
          );
          final bubbleHits = BoxHitTestResult();
          expect(
            surface.hitTest(
              bubbleHits,
              position: bubbleOffset + const Offset(50, 50),
            ),
            isTrue,
          );
          expect(
            surface.hitTest(
              BoxHitTestResult(),
              position: const Offset(-10, -10),
            ),
            isFalse,
          );
          final node = tester.getSemantics(
            find.bySemanticsLabel('Flying link'),
          );
          expect(node.rect.size, const Size(100, 100));
          expect(tester.takeException(), isNull);
          semantics.dispose();
        },
      );
    }
  }

  for (final text in [
    'Hello',
    'A longer message\nwith several lines\nof text',
  ]) {
    testWidgets(
      'send background contracts without changing text layout: $text',
      (tester) async {
        final key = GlobalKey();
        Rect? origin;
        Future<void> draw(double progress, {double? widthProgress}) async {
          await tester.pumpWidget(
            MaterialApp(
              home: Center(
                child: RepaintBoundary(
                  key: key,
                  child: SizedBox(
                    width: 300,
                    height: 120,
                    child: ColoredBox(
                      color: Colors.white,
                      child: Align(
                        alignment: Alignment.topLeft,
                        child: LocalSendProgress(
                          progress: progress,
                          widthProgress: widthProgress,
                          startOffset: 0,
                          origin: origin,
                          child: LocalSendBubbleSurface(
                            color: const Color(0xFFFF0000),
                            shape: const RoundedRectangleBorder(),
                            child: Padding(
                              padding: const EdgeInsets.all(8),
                              child: Text(
                                text,
                                style: const TextStyle(fontSize: 12),
                                textWidthBasis: TextWidthBasis.longestLine,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          );
        }

        Future<int> greenAt(int x) async {
          final boundary =
              key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
          return (await tester.runAsync(() async {
            final image = await boundary.toImage();
            final bytes = (await image.toByteData(
              format: ui.ImageByteFormat.rawRgba,
            ))!;
            final green = bytes.getUint8((4 * 300 + x) * 4 + 1);
            image.dispose();
            return green;
          }))!;
        }

        await draw(0);
        final textRect = tester.getRect(find.text(text));
        final finalWidth = tester
            .getSize(find.byType(LocalSendBubbleSurface))
            .width;
        expect(finalWidth, lessThan(280));
        expect(await greenAt(280), 0);
        await draw(0.5);
        expect(tester.getRect(find.text(text)), textRect);
        final middle = ((300 + finalWidth) / 2).floor();
        expect(await greenAt(middle - 2), 0);
        expect(await greenAt(middle + 2), 255);
        await draw(1);
        expect(tester.getRect(find.text(text)), textRect);
        expect(await greenAt(finalWidth.ceil() + 2), 255);
        await draw(1, widthProgress: 1.012);
        expect(
          tester.getSize(find.byType(LocalSendBubbleSurface)).width,
          finalWidth,
        );
        // The fractional spring edge can partially cover this pixel.
        expect(await greenAt(finalWidth.floor() - 1), greaterThan(128));
        expect(
          await greenAt((finalWidth - (300 - finalWidth) * 0.012).floor() - 2),
          0,
        );
        await draw(1);
        expect(await greenAt(finalWidth.floor() - 1), 0);
        final boundary = key.currentContext!.findRenderObject()! as RenderBox;
        origin = Rect.fromLTWH(
          20,
          50,
          250,
          30,
        ).shift(boundary.localToGlobal(Offset.zero));
        await draw(0);
        final image = await tester.runAsync(() async {
          final result = await (boundary as RenderRepaintBoundary).toImage();
          final bytes = (await result.toByteData(
            format: ui.ImageByteFormat.rawRgba,
          ))!;
          final inside = bytes.getUint8((55 * 300 + 25) * 4 + 1);
          final outside = bytes.getUint8((55 * 300 + 275) * 4 + 1);
          result.dispose();
          return (inside, outside);
        });
        expect(image, (0, 255));
        // Width can settle while the bubble is still partway up its flight.
        await draw(0.5, widthProgress: 1);
        final staged = await tester.runAsync(() async {
          final result = await (boundary as RenderRepaintBoundary).toImage();
          final bytes = (await result.toByteData(
            format: ui.ImageByteFormat.rawRgba,
          ))!;
          final inside = bytes.getUint8((28 * 300 + 12) * 4 + 1);
          final outside = bytes.getUint8(
            (28 * 300 + finalWidth.ceil() + 14) * 4 + 1,
          );
          result.dispose();
          return (inside, outside);
        });
        expect(staged, (0, 255));
        await draw(1);
        expect(tester.takeException(), isNull);
      },
    );
  }
}
