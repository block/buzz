import 'dart:convert';
import 'dart:async';

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

  group('MediaGetAuthService memoization', () {
    test('repeated calls return byte-identical headers', () {
      final nsec = nostr.Keys.generate().nsec;
      final auth = _auth(nsec: nsec);
      final first = auth.headersFor(_mediaUrl);
      final second = auth.headersFor(_mediaUrl);
      expect(first, isNotEmpty);
      expect(identical(first, second), isTrue);
    });

    test('re-signs only at the refresh margin before expiry', () {
      final nsec = nostr.Keys.generate().nsec;
      var current = DateTime.utc(2026, 7, 21, 12);
      final auth = _auth(nsec: nsec, now: () => current);

      final first = auth.headersFor(_mediaUrl);
      // 600s lifetime - 60s margin = re-sign boundary at +540s.
      current = current.add(const Duration(seconds: 539));
      expect(identical(auth.headersFor(_mediaUrl), first), isTrue);

      current = current.add(const Duration(seconds: 2));
      final refreshed = auth.headersFor(_mediaUrl);
      expect(identical(refreshed, first), isFalse);
      expect(refreshed['Authorization'], isNot(first['Authorization']));
    });

    test('non-relay URLs get no headers even with a key', () {
      final nsec = nostr.Keys.generate().nsec;
      final auth = _auth(nsec: nsec);
      expect(auth.headersFor('https://elsewhere.com/media/abc.png'), isEmpty);
      expect(auth.headersFor('$_relayBase/not-media/abc.png'), isEmpty);
    });
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
