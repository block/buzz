import 'dart:ui' as ui;
import 'dart:convert';
import 'dart:async';
import 'dart:typed_data';

import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;

void main() {
  testWidgets('bounds remote avatar decoding before painting the native PNG', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final recorder = ui.PictureRecorder();
      Canvas(recorder).drawColor(Colors.blue, BlendMode.src);
      final picture = recorder.endRecording();
      final image = await picture.toImage(1440, 720);
      final bytes = (await image.toByteData(
        format: ui.ImageByteFormat.png,
      ))!.buffer.asUint8List();
      image.dispose();
      picture.dispose();
      final provider = _RecordingImage(bytes);
      final png = await nativeAvatarImage(
        url: 'https://example.com/avatar.png',
        initial: 'A',
        background: Colors.white,
        foreground: Colors.black,
        networkImages: (_) => [provider],
      );
      expect(png, isNotNull);
      expect(provider.decodedWidth, lessThanOrEqualTo(72));
      expect(provider.decodedHeight, lessThanOrEqualTo(72));
      expect(provider.decodedWidth, greaterThan(0));
    });
  });

  testWidgets('native relay avatars load only thumbnails, like MediaImage', (
    tester,
  ) async {
    await tester.runAsync(() async {
      MediaImage.debugResetMissingThumbnails();
      final recorder = ui.PictureRecorder();
      Canvas(recorder).drawColor(Colors.blue, BlendMode.src);
      final picture = recorder.endRecording();
      final image = await picture.toImage(8, 8);
      final blue = (await image.toByteData(
        format: ui.ImageByteFormat.png,
      ))!.buffer.asUint8List();
      image.dispose();
      picture.dispose();

      const relay = 'https://relay.example';
      final jpegOnly = '$relay/media/${'a1' * 32}.png';
      final missing = '$relay/media/${'b2' * 32}.png';
      final broken = '$relay/media/${'c3' * 32}.png';
      String thumb(String url, String ext) =>
          url.replaceFirst('.png', '.thumb.$ext');
      final requested = <String>[];
      final auth = MediaGetAuthService(baseUrl: relay, nsec: null);
      final client = http_testing.MockClient((request) async {
        final url = request.url.toString();
        requested.add(url);
        if (url == thumb(jpegOnly, 'jpg')) {
          return http.Response.bytes(blue, 200);
        }
        if (url == thumb(broken, 'webp')) return http.Response('', 500);
        return http.Response('', 404);
      });
      Future<bool> paintsImage(String url) async {
        final png = await nativeAvatarImage(
          url: url,
          initial: 'A',
          background: Colors.black,
          foreground: Colors.white,
          networkImages: (url) =>
              smallMediaImageProviders(url: url, auth: auth, client: client),
        );
        final codec = await ui.instantiateImageCodec(png!);
        final frame = await codec.getNextFrame();
        final pixels = (await frame.image.toByteData())!;
        frame.image.dispose();
        codec.dispose();
        // Left of center, clear of the initial: blue artwork or black.
        return pixels.getUint8((36 * 72 + 10) * 4 + 2) > 200;
      }

      // A 404 moves to the JPEG thumbnail; the original is never fetched.
      expect(await paintsImage(jpegOnly), isTrue);
      expect(requested, [thumb(jpegOnly, 'webp'), thumb(jpegOnly, 'jpg')]);
      // With no thumbnail at all, the initials show.
      requested.clear();
      expect(await paintsImage(missing), isFalse);
      expect(requested, [thumb(missing, 'webp'), thumb(missing, 'jpg')]);
      // Known-missing thumbnails are skipped on the next rasterization.
      requested.clear();
      expect(await paintsImage(missing), isFalse);
      expect(requested, isEmpty);
      // Any other failure stops at the initials.
      expect(await paintsImage(broken), isFalse);
      expect(requested, [thumb(broken, 'webp')]);
    });
  });

  for (final scenario in [
    'cancel',
    'timeout then cancel',
    'timeout then finish',
    'timeout without owner',
  ]) {
    testWidgets('native avatar $scenario', (tester) async {
      final started = Completer<void>();
      final aborted = Completer<void>();
      final response = Completer<http.StreamedResponse>();
      final cancelled = ValueNotifier(false);
      var requests = 0;
      final client = _StreamingClient((request) async {
        requests++;
        (request as http.AbortableRequest).abortTrigger!.then((_) {
          if (!response.isCompleted) {
            if (!aborted.isCompleted) aborted.complete();
            response.completeError(http.RequestAbortedException(request.url));
          }
        });
        if (!started.isCompleted) started.complete();
        return response.future;
      });
      final provider = MediaImageProvider(
        url: 'https://relay.example/media/held-$scenario.png',
        auth: MediaGetAuthService(baseUrl: 'https://relay.example', nsec: null),
        client: client,
      );
      Future<Uint8List?> render() => nativeAvatarImage(
        url: provider.url,
        cancelled: scenario == 'timeout without owner' ? null : cancelled,
        initial: 'A',
        background: Colors.black,
        foreground: Colors.white,
        networkImages: (_) => [provider],
      );
      final pending = render();
      await tester.pump();
      expect(started.isCompleted, isTrue);
      if (scenario != 'cancel') {
        await tester.pump(const Duration(seconds: 11));
        expect(aborted.isCompleted, isFalse);
      }
      if (scenario.contains('cancel')) {
        cancelled.value = true;
        await tester.pump();
        expect(aborted.isCompleted, isTrue);
        await tester.runAsync(() async {
          await pending;
        });
      } else {
        await tester.runAsync(() async {
          await pending;
        });
        var loaded = false;
        final stream = ResizeImage(
          provider,
          width: 72,
          height: 72,
          policy: ResizeImagePolicy.fit,
        ).resolve(ImageConfiguration.empty);
        final listener = ImageStreamListener((_, _) {
          loaded = true;
        });
        stream.addListener(listener);
        final bytes = base64Decode(
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAA'
          'AAYAAjCB0C8AAAAASUVORK5CYII=',
        );
        response.complete(http.StreamedResponse(Stream.value(bytes), 200));
        // Drain engine decode events and the test zone's microtasks until the
        // real image stream settles, without advancing the request's timeout.
        while (!loaded) {
          await tester.runAsync(() => Future<void>(() {}));
          await tester.pump();
        }
        stream.removeListener(listener);
        await tester.runAsync(() async {
          expect(await render(), isNotNull);
          expect(await render(), isNotNull);
        });
        expect(requests, 1);
        expect(aborted.isCompleted, isFalse);
        cancelled.value = true;
        await tester.pump();
        expect(aborted.isCompleted, isFalse);
      }
      cancelled.dispose();
      client.close();
    });
  }

  for (final size in [const Size(1440, 720), const Size(720, 1440)]) {
    testWidgets('bounds inline raster decode at $size before native painting', (
      tester,
    ) async {
      await tester.runAsync(() async {
        final recorder = ui.PictureRecorder();
        Canvas(recorder).drawColor(Colors.blue, BlendMode.src);
        final picture = recorder.endRecording();
        final image = await picture.toImage(
          size.width.toInt(),
          size.height.toInt(),
        );
        final bytes = (await image.toByteData(
          format: ui.ImageByteFormat.png,
        ))!.buffer.asUint8List();
        image.dispose();
        picture.dispose();
        final decodedSizes = <Size>[];
        final previous = ui.Image.onCreate;
        ui.Image.onCreate = (image) {
          decodedSizes.add(
            Size(image.width.toDouble(), image.height.toDouble()),
          );
          previous?.call(image);
        };
        try {
          final png = await nativeAvatarImage(
            url: 'data:image/png;base64,${base64Encode(bytes)}',
            initial: 'A',
            background: Colors.white,
            foreground: Colors.black,
            networkImages: (_) => throw StateError('Unexpected network image'),
          );
          expect(png, isNotNull);
          // The first image is the codec frame, before the final 72px canvas.
          expect(decodedSizes.length, greaterThanOrEqualTo(2));
          expect(decodedSizes.first.width, lessThanOrEqualTo(72));
          expect(decodedSizes.first.height, lessThanOrEqualTo(72));
          expect(decodedSizes.first.aspectRatio, size.aspectRatio);
          expect(decodedSizes.first.longestSide, 72);
        } finally {
          ui.Image.onCreate = previous;
        }
      });
    });
  }

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
          networkImages: (_) => throw StateError('Unexpected network image'),
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

class _RecordingImage extends MemoryImage {
  _RecordingImage(super.bytes);
  final decodedSizes = <Size>[];
  double get decodedWidth => decodedSizes.single.width;
  double get decodedHeight => decodedSizes.single.height;

  @override
  ImageStreamCompleter loadImage(
    MemoryImage key,
    ImageDecoderCallback decode,
  ) => MultiFrameImageStreamCompleter(codec: _decode(decode), scale: 1);

  Future<ui.Codec> _decode(ImageDecoderCallback decode) async {
    final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
    final codec = await decode(buffer);
    final frame = await codec.getNextFrame();
    decodedSizes.add(
      Size(frame.image.width.toDouble(), frame.image.height.toDouble()),
    );
    frame.image.dispose();
    return codec;
  }
}

class _StreamingClient extends http.BaseClient {
  _StreamingClient(this.onSend);
  final Future<http.StreamedResponse> Function(http.BaseRequest) onSend;
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) =>
      onSend(request);
}
