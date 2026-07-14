import 'dart:convert';

import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const svg =
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">'
      '<text x="16" y="24" text-anchor="middle">🦝</text></svg>';

  Widget subject(String? imageUrl) => MaterialApp(
    home: AvatarImage(
      imageUrl: imageUrl,
      radius: 16,
      fallback: const Text('R'),
    ),
  );

  testWidgets('renders raccoon percent-encoded SVG data avatar', (
    tester,
  ) async {
    const raccoonAvatar =
        'data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%22512%22%20height%3D%22512%22%20viewBox%3D%220%200%20512%20512%22%3E%3Crect%20width%3D%22512%22%20height%3D%22512%22%20rx%3D%22256%22%20fill%3D%22%233399FF%22%2F%3E%3Ctext%20x%3D%2250%25%22%20y%3D%2256%25%22%20dominant-baseline%3D%22middle%22%20text-anchor%3D%22middle%22%20font-size%3D%22258%22%3E%F0%9F%A6%9D%3C%2Ftext%3E%3C%2Fsvg%3E';
    await tester.pumpWidget(subject(raccoonAvatar));
    await tester.pumpAndSettle();

    final emoji = tester.widget<Text>(find.text('🦝'));
    expect(emoji.style?.height, 1);
    expect(find.byType(SvgPicture), findsNothing);
    expect(find.text('R'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('renders base64 SVG data avatar', (tester) async {
    await tester.pumpWidget(
      subject('data:image/svg+xml;base64,${base64Encode(utf8.encode(svg))}'),
    );

    expect(find.byType(SvgPicture), findsOneWidget);
  });

  testWidgets('centers fallback when no avatar is configured', (tester) async {
    await tester.pumpWidget(subject(null));

    final avatarCenter = tester.getCenter(find.byType(CircleAvatar));
    final fallbackCenter = tester.getCenter(find.text('R'));
    expect(fallbackCenter, avatarCenter);
  });

  testWidgets('uses fallback for malformed image data', (tester) async {
    await tester.pumpWidget(subject('data:image/svg+xml;base64,%%%'));

    expect(find.text('R'), findsOneWidget);
  });

  testWidgets('reuses parsed raster bytes across parent rebuilds', (
    tester,
  ) async {
    const png =
        'data:image/png;base64,'
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';
    await tester.pumpWidget(subject(png));
    final firstBytes = tester.widget<Image>(find.byType(Image)).image;

    await tester.pumpWidget(subject(png));
    final rebuiltBytes = tester.widget<Image>(find.byType(Image)).image;

    final firstMemory = firstBytes as MemoryImage;
    final rebuiltMemory = rebuiltBytes as MemoryImage;
    expect(rebuiltMemory.bytes, same(firstMemory.bytes));
  });
}
