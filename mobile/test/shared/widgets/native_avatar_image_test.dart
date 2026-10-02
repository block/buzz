import 'dart:ui' as ui;

import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final emoji in ['🥳', '🦝', '👩🏽‍💻']) {
    testWidgets('centers painted bounds of $emoji in the native avatar', (
      tester,
    ) async {
      await tester.runAsync(() async {
        final png = await nativeAvatarImage(
          url: emojiAvatarDataUrl(emoji, 0xFF000000),
          initial: 'K',
          background: Colors.black,
          foreground: Colors.white,
          networkImage: (_) => throw StateError('Unexpected network image'),
        );
        expect(png, isNotNull);
        final codec = await ui.instantiateImageCodec(png!);
        final frame = await codec.getNextFrame();
        final pixels = (await frame.image.toByteData())!;
        var minX = 72;
        var minY = 72;
        var maxX = -1;
        var maxY = -1;
        for (var y = 0; y < 72; y++) {
          for (var x = 0; x < 72; x++) {
            final index = (y * 72 + x) * 4;
            if (pixels.getUint8(index) +
                    pixels.getUint8(index + 1) +
                    pixels.getUint8(index + 2) <
                48) {
              continue;
            }
            if (x < minX) minX = x;
            if (x > maxX) maxX = x;
            if (y < minY) minY = y;
            if (y > maxY) maxY = y;
          }
        }
        frame.image.dispose();
        codec.dispose();
        expect(maxX, greaterThan(minX));
        expect((minX + maxX + 1) / 2, closeTo(36, 0.5));
        expect((minY + maxY + 1) / 2, closeTo(36, 0.5));
      });
    });
  }
}
