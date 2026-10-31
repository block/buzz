import 'dart:convert';

import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:buzz/shared/widgets/avatar_shape.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:buzz/shared/emoji/native_emoji_glyph.dart';
import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;

void main() {
  const svg =
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">'
      '<text x="16" y="24" text-anchor="middle">🦝</text></svg>';

  Widget subject(
    String? imageUrl, {
    Color? backgroundColor,
    bool isAgent = false,
  }) => ProviderScope(
    child: MaterialApp(
      home: AvatarImage(
        imageUrl: imageUrl,
        radius: 16,
        backgroundColor: backgroundColor,
        fallback: const Text('R'),
        isAgent: isAgent,
      ),
    ),
  );

  testWidgets('only animated agent artwork shrinks inside its unchanged frame', (
    tester,
  ) async {
    const poster = 'https://relay.example/poster.png';
    final animated =
        '$poster#buzz-anim=${Uri.encodeComponent('https://relay.example/animation.png')}';
    const preset =
        'https://dwwgwmfqqjotj.cloudfront.net/avatars/20260925T154625166Z/poster/pollies/pollies-10.png';
    for (final url in [poster, animated, preset]) {
      await tester.pumpWidget(subject(url, isAgent: true));
      final clip = find.byType(ClipPath);
      final scale = find.descendant(of: clip, matching: find.byType(Transform));
      if (url != poster) {
        expect(scale, findsOneWidget);
        expect(tester.widget<Transform>(scale).transform.entry(0, 0), 0.85);
        final content = find.byType(AvatarImageContent);
        expect(tester.getSize(content), tester.getSize(clip));
      } else {
        expect(scale, findsNothing);
      }
    }
  });

  testWidgets('small relay avatars load the relay thumbnail', (tester) async {
    const relay = 'https://relay.example';
    final hash = 'ab' * 32;
    final original = '$relay/media/$hash.jpg';
    final requested = <String>[];
    final client = http_testing.MockClient((request) async {
      requested.add(request.url.toString());
      return http.Response('', 404);
    });
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          mediaGetAuthServiceProvider.overrideWithValue(
            MediaGetAuthService(baseUrl: relay, nsec: null),
          ),
          mediaHttpClientProvider.overrideWithValue(client),
        ],
        child: MaterialApp(
          home: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              // 40pt = 120px, within the thumbnail.
              AvatarImage(
                imageUrl: original,
                radius: 20,
                fallback: const Text('R'),
              ),
              // 120pt = 360px, larger than the thumbnail.
              AvatarImage(
                imageUrl: original,
                radius: 60,
                fallback: const Text('R'),
              ),
            ],
          ),
        ),
      ),
    );

    final shown = tester.widgetList<Image>(find.byType(Image)).map((image) {
      final provider = (image.image as ResizeImage).imageProvider;
      return (provider as MediaImageProvider).url;
    });
    expect(shown, ['$relay/media/$hash.thumb.jpg', original]);
  });

  testWidgets('a missing thumbnail falls back to the original', (tester) async {
    const relay = 'https://relay.example';
    final missing = '$relay/media/${'cd' * 32}.jpg';
    final broken = '$relay/media/${'ef' * 32}.jpg';
    final requested = <String>[];
    final client = http_testing.MockClient((request) async {
      final url = request.url.toString();
      requested.add(url);
      if (url.startsWith(broken.replaceFirst('.jpg', ''))) {
        return http.Response('', 500);
      }
      return http.Response('', 404);
    });
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          mediaGetAuthServiceProvider.overrideWithValue(
            MediaGetAuthService(baseUrl: relay, nsec: null),
          ),
          mediaHttpClientProvider.overrideWithValue(client),
        ],
        child: MaterialApp(
          home: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final url in [missing, broken])
                AvatarImage(
                  imageUrl: url,
                  radius: 20,
                  fallback: Text(url == missing ? 'M' : 'B'),
                ),
            ],
          ),
        ),
      ),
    );
    for (var i = 0; i < 4; i++) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 20)),
      );
      await tester.pump();
    }

    String thumb(String url) => url.replaceFirst('.jpg', '.thumb.jpg');
    // A 404 retries the original; any other failure shows the fallback.
    expect(requested, containsAll([thumb(missing), missing, thumb(broken)]));
    expect(requested, isNot(contains(broken)));
    expect(find.text('B'), findsOneWidget);
  });

  testWidgets('clips agents to the desktop squircle', (tester) async {
    await tester.pumpWidget(subject(null, isAgent: true));

    expect(find.byType(CircleAvatar), findsNothing);
    final clip = tester.widget<ClipPath>(find.byType(ClipPath));
    expect(
      (clip.clipper! as ShapeBorderClipper).shape,
      avatarShape(isAgent: true),
    );
  });

  testWidgets('renders raccoon percent-encoded SVG data avatar', (
    tester,
  ) async {
    const raccoonAvatar =
        'data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%22512%22%20height%3D%22512%22%20viewBox%3D%220%200%20512%20512%22%3E%3Crect%20width%3D%22512%22%20height%3D%22512%22%20rx%3D%22256%22%20fill%3D%22%233399FF%22%2F%3E%3Ctext%20x%3D%2250%25%22%20y%3D%2256%25%22%20dominant-baseline%3D%22middle%22%20text-anchor%3D%22middle%22%20font-size%3D%22258%22%3E%F0%9F%A6%9D%3C%2Ftext%3E%3C%2Fsvg%3E';
    await tester.pumpWidget(subject(raccoonAvatar));
    await tester.pumpAndSettle();

    final emoji = tester.widget<Text>(find.text('🦝'));
    expect(emoji.style?.height, 1);
    final glyph = tester.widget<NativeEmojiGlyph>(
      find.byType(NativeEmojiGlyph),
    );
    expect(glyph.size, closeTo(32 * 258 / 512, 0.001));
    expect(glyph.opticalBoxSize, glyph.size);
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

  testWidgets('renders animated-avatar posters without an opaque background', (
    tester,
  ) async {
    const posterUrl = 'https://relay.example/media/poster.png';
    const animationUrl = 'https://relay.example/media/animation.png';
    final animatedUrl =
        '$posterUrl#buzz-anim=${Uri.encodeComponent(animationUrl)}';

    await tester.pumpWidget(subject(animatedUrl, backgroundColor: Colors.red));

    expect(
      tester.widget<CircleAvatar>(find.byType(CircleAvatar)).backgroundColor,
      Colors.transparent,
    );
    expect(
      tester
          .widget<AvatarImageContent>(find.byType(AvatarImageContent))
          .imageUrl,
      posterUrl,
    );
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
