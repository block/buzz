import 'dart:ui' as ui;
import 'package:buzz/features/channels/local_message_send_transition.dart';
import 'package:buzz/features/channels/local_send_bubble_surface.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final text in [
    'Hello',
    'A longer message\nwith several lines\nof text',
  ]) {
    testWidgets(
      'send background contracts without changing text layout: $text',
      (tester) async {
        final key = GlobalKey();
        Future<void> draw(double progress) async {
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
        expect(tester.takeException(), isNull);
      },
    );
  }
}
