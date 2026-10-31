import 'dart:convert';
import 'dart:async';
import 'dart:typed_data';

import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

// Minimal valid 1x1 transparent PNG.
final _pngBytes = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAA'
  'AAYAAjCB0C8AAAAASUVORK5CYII=',
);

const _relayBase = 'https://relay.example.com';
const _mediaUrl = '$_relayBase/media/abc123.png';

MediaGetAuthService _auth({String? nsec, DateTime Function()? now}) =>
    MediaGetAuthService(baseUrl: _relayBase, nsec: nsec, now: now);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    MediaImageProvider.debugResetCooldowns();
    MediaImageProvider.debugNow = DateTime.now;
    PaintingBinding.instance.imageCache.clear();
    PaintingBinding.instance.imageCache.clearLiveImages();
  });

  group('MediaGetAuthService mint-per-request', () {
    test('consecutive calls return distinct headers (mint-per-request)', () {
      final nsec = nostr.Keys.generate().nsec;
      final auth = _auth(nsec: nsec);
      final first = auth.headersFor(_mediaUrl);
      final second = auth.headersFor(_mediaUrl);
      expect(first, isNotEmpty);
      // With lifetime == margin == 60s, refreshAt == signedAt, so the cache
      // never hits: each call produces a fresh proof.
      expect(identical(first, second), isFalse);
      expect(second['Authorization'], isNot(first['Authorization']));
    });

    test('each call produces a fresh token regardless of elapsed time', () {
      final nsec = nostr.Keys.generate().nsec;
      var current = DateTime.utc(2026, 7, 21, 12);
      final auth = _auth(nsec: nsec, now: () => current);

      final first = auth.headersFor(_mediaUrl);
      // No elapsed time: still mints fresh.
      final atZero = auth.headersFor(_mediaUrl);
      expect(identical(atZero, first), isFalse);
      expect(atZero['Authorization'], isNot(first['Authorization']));

      // Advance clock: still mints fresh.
      current = current.add(const Duration(seconds: 30));
      final at30 = auth.headersFor(_mediaUrl);
      expect(identical(at30, first), isFalse);
      expect(at30['Authorization'], isNot(first['Authorization']));
    });

    test('non-relay URLs get no headers even with a key', () {
      final nsec = nostr.Keys.generate().nsec;
      final auth = _auth(nsec: nsec);
      expect(auth.headersFor('https://elsewhere.com/media/abc.png'), isEmpty);
      expect(auth.headersFor('$_relayBase/not-media/abc.png'), isEmpty);
    });
  });

  group('relayThumbnailUrl', () {
    final hash = 'ab' * 32;
    final auth = _auth();

    test('maps relay blobs of any format to the thumbnail', () {
      for (final ext in ['.jpg', '.jpeg', '.png', '.webp', '.gif', '']) {
        expect(
          relayThumbnailUrl('$_relayBase/media/$hash$ext', auth),
          '$_relayBase/media/$hash.thumb.jpg',
        );
      }
    });

    test('leaves everything else alone', () {
      for (final url in [
        'https://elsewhere.example/media/$hash.jpg',
        '$_relayBase/media/$hash.thumb.jpg',
        '$_relayBase/media/$hash.jpg?v=2',
        '$_relayBase/media/not-a-hash.jpg',
        '$_relayBase/avatars/$hash.jpg',
        '$_relayBase/media/$hash.JPG',
      ]) {
        expect(relayThumbnailUrl(url, auth), isNull, reason: url);
      }
    });
  });

  test('remembers a bounded number of missing thumbnails', () {
    for (var i = 0; i < 1000; i++) {
      MediaImage.debugRememberMissingThumbnail(
        '$_relayBase/media/$i.thumb.jpg',
      );
    }
    expect(MediaImage.debugMissingThumbnailCount, 256);
  });

  group('MediaImageProvider cache identity', () {
    test('equal url + same auth service => equal keys', () {
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      final client = http.Client();
      addTearDown(client.close);
      final a = MediaImageProvider(url: _mediaUrl, auth: auth, client: client);
      final b = MediaImageProvider(url: _mediaUrl, auth: auth, client: client);
      expect(a, equals(b));
      expect(a.hashCode, equals(b.hashCode));
    });

    test('different auth service (relay/account switch) => unequal keys', () {
      final client = http.Client();
      addTearDown(client.close);
      final a = MediaImageProvider(
        url: _mediaUrl,
        auth: _auth(nsec: nostr.Keys.generate().nsec),
        client: client,
      );
      final b = MediaImageProvider(
        url: _mediaUrl,
        auth: _auth(nsec: nostr.Keys.generate().nsec),
        client: client,
      );
      expect(a, isNot(equals(b)));
    });

    test('transport client is not part of identity', () {
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      final c1 = http.Client();
      final c2 = http.Client();
      addTearDown(c1.close);
      addTearDown(c2.close);
      expect(
        MediaImageProvider(url: _mediaUrl, auth: auth, client: c1),
        equals(MediaImageProvider(url: _mediaUrl, auth: auth, client: c2)),
      );
    });
  });

  group('MediaImageProvider fetching', () {
    test(
      'oversized image streams abort before decode and preserve the client',
      () async {
        for (final declared in [true, false]) {
          var cancelled = false;
          var aborted = false;
          var delivered = false;
          final controller = StreamController<List<int>>(
            onCancel: () => cancelled = true,
          );
          final client = _StreamingClient((request) async {
            if (request.url.path.endsWith('valid.png')) {
              return http.StreamedResponse(Stream.value(_pngBytes), 200);
            }
            (request as http.AbortableRequest).abortTrigger!.then((_) {
              aborted = true;
            });
            // Never finish: the loader must stop as soon as the cap is crossed.
            controller.add(Uint8List(MediaImageProvider.maxResponseBytes));
            controller.add([0]);
            return http.StreamedResponse(
              controller.stream,
              200,
              contentLength: declared
                  ? MediaImageProvider.maxResponseBytes + 1
                  : null,
            );
          });
          final provider = MediaImageProvider(
            url: 'https://avatar.example/$declared.png',
            auth: _auth(),
            client: client,
            onBytesLoaded: (_) => delivered = true,
          );
          final error = await _waitError(
            provider.resolve(ImageConfiguration.empty),
          ).timeout(const Duration(seconds: 2));
          expect(error, isA<FormatException>());
          expect(cancelled, isTrue);
          expect(aborted, isTrue);
          expect(delivered, isFalse);
          await controller.close();
          await _wait(
            MediaImageProvider(
              url: 'https://avatar.example/valid.png',
              auth: _auth(),
              client: client,
            ).resolve(ImageConfiguration.empty),
          );
          PaintingBinding.instance.imageCache.clear();
        }
      },
    );

    test('resolves one fetch for repeated resolves of the same key', () async {
      var fetches = 0;
      final client = http_testing.MockClient((request) async {
        fetches += 1;
        return http.Response.bytes(_pngBytes, 200);
      });
      final auth = _auth(nsec: nostr.Keys.generate().nsec);

      for (var i = 0; i < 3; i++) {
        final provider = MediaImageProvider(
          url: _mediaUrl,
          auth: auth,
          client: client,
        );
        final completer = provider.resolve(ImageConfiguration.empty);
        await _wait(completer);
      }
      expect(fetches, 1);
    });

    test('one URL shown at two decode sizes downloads once', () async {
      var fetches = 0;
      final response = Completer<void>();
      final client = http_testing.MockClient((request) async {
        fetches += 1;
        await response.future;
        return http.Response.bytes(_pngBytes, 200);
      });
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      ImageStream resolveAt(int width) => ResizeImage(
        MediaImageProvider(url: _mediaUrl, auth: auth, client: client),
        width: width,
      ).resolve(ImageConfiguration.empty);

      final small = _wait(resolveAt(40));
      final large = _wait(resolveAt(120));
      await pumpEventQueue();
      response.complete();
      await Future.wait([small, large]);

      expect(fetches, 1);
    });

    test('sends auth headers with the fetch', () async {
      Map<String, String>? seen;
      final client = http_testing.MockClient((request) async {
        seen = request.headers;
        return http.Response.bytes(_pngBytes, 200);
      });
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      final provider = MediaImageProvider(
        url: _mediaUrl,
        auth: auth,
        client: client,
      );
      await _wait(provider.resolve(ImageConfiguration.empty));
      expect(seen?['Authorization'], startsWith('Nostr '));
    });

    test('failed URL is not refetched until cooldown elapses', () async {
      var fetches = 0;
      final client = http_testing.MockClient((request) async {
        fetches += 1;
        return http.Response('rate limited', 429);
      });
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      var current = DateTime.utc(2026, 7, 21, 12);
      MediaImageProvider.debugNow = () => current;

      Future<Object?> attempt() async {
        final provider = MediaImageProvider(
          url: _mediaUrl,
          auth: auth,
          client: client,
        );
        return _waitError(provider.resolve(ImageConfiguration.empty));
      }

      expect(await attempt(), isA<NetworkImageLoadException>());
      expect(fetches, 1);

      // Within cooldown: suppressed, no network call.
      expect(await attempt(), isA<MediaImageCooldownException>());
      expect(fetches, 1);

      // After cooldown: retried.
      current = current.add(const Duration(seconds: 31));
      expect(await attempt(), isA<NetworkImageLoadException>());
      expect(fetches, 2);
    });

    test('honors Retry-After on 429 (capped)', () async {
      var fetches = 0;
      final client = http_testing.MockClient((request) async {
        fetches += 1;
        return http.Response(
          'rate limited',
          429,
          headers: {'retry-after': '120'},
        );
      });
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      var current = DateTime.utc(2026, 7, 21, 12);
      MediaImageProvider.debugNow = () => current;

      Future<Object?> attempt() async {
        final provider = MediaImageProvider(
          url: _mediaUrl,
          auth: auth,
          client: client,
        );
        return _waitError(provider.resolve(ImageConfiguration.empty));
      }

      await attempt();
      expect(fetches, 1);

      current = current.add(const Duration(seconds: 60));
      expect(await attempt(), isA<MediaImageCooldownException>());
      expect(fetches, 1);

      current = current.add(const Duration(seconds: 61));
      await attempt();
      expect(fetches, 2);
    });
  });
}

Future<void> _wait(ImageStream stream) {
  final done = Completer<void>();
  late final ImageStreamListener listener;
  listener = ImageStreamListener(
    (image, sync) {
      image.dispose();
      stream.removeListener(listener);
      if (!done.isCompleted) done.complete();
    },
    onError: (error, stack) {
      stream.removeListener(listener);
      if (!done.isCompleted) done.completeError(error, stack);
    },
  );
  stream.addListener(listener);
  return done.future;
}

Future<Object?> _waitError(ImageStream stream) async {
  try {
    await _wait(stream);
    return null;
  } catch (e) {
    return e;
  }
}

class _StreamingClient extends http.BaseClient {
  _StreamingClient(this.respond);
  final Future<http.StreamedResponse> Function(http.BaseRequest) respond;
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) =>
      respond(request);
}
