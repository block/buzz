import 'dart:ui' as ui;

import 'package:buzz/shared/widgets/top_edge_fade.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('fades only the top edge and preserves high contrast', (
    tester,
  ) async {
    for (final highContrast in [false, true]) {
      final key = GlobalKey();
      await tester.pumpWidget(
        MediaQuery(
          data: MediaQueryData(highContrast: highContrast),
          child: Center(
            child: RepaintBoundary(
              key: key,
              child: const SizedBox(
                width: 64,
                height: 64,
                child: TopEdgeFade(child: ColoredBox(color: Colors.red)),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      await tester.runAsync(() async {
        final boundary =
            key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
        final image = await boundary.toImage();
        final data = (await image.toByteData(
          format: ui.ImageByteFormat.rawRgba,
        ))!;
        int alpha(int y) => data.getUint8((y * 64 + 32) * 4 + 3);
        if (highContrast) {
          expect(alpha(0), 255);
        } else {
          expect(alpha(0), lessThan(20));
          expect(alpha(8), inInclusiveRange(100, 160));
        }
        expect(alpha(16), 255);
        expect(alpha(63), 255);
        image.dispose();
      });
    }
  });
}
