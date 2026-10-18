import 'dart:async';
import 'dart:convert';

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

  group('background media auth', () {
    test(
      'mints fresh valid server-scoped proofs for concurrent requests',
      () async {
        final keys = nostr.Keys.generate();
        final signedAt = DateTime.utc(2026, 10, 17, 12);
        final auth = _auth(nsec: keys.nsec, now: () => signedAt);
        final headers = await Future.wait([
          auth.headersForAsync(_mediaUrl),
          auth.headersForAsync(_mediaUrl),
        ]);
        final events = headers.map((header) {
          final encoded = header['Authorization']!.substring('Nostr '.length);
          return nostr.Event.fromJson(
            utf8.decode(base64Url.decode(base64Url.normalize(encoded))),
          );
        }).toList();
        expect(events[0].sig, isNot(events[1].sig));
        for (final event in events) {
          expect(event.isValid(), isTrue);
          expect(event.pubkey, keys.public);
          expect(event.kind, 24242);
          expect(event.tags, contains(equals(['t', 'get'])));
          expect(event.tags, contains(equals(['server', 'relay.example.com'])));
          final expiration = int.parse(
            event.tags.singleWhere((tag) => tag.first == 'expiration')[1],
          );
          expect(expiration, signedAt.millisecondsSinceEpoch ~/ 1000 + 60);
        }
      },
    );

    test('bounds queued signing and admits requests after it drains', () async {
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      // Enqueue in one turn, before a worker can finish. Record every failure
      // immediately so overflow futures cannot become unhandled errors.
      final requests = List.generate(
        80,
        (_) => auth
            .headersForAsync(_mediaUrl)
            .then<Object>(
              (headers) => headers,
              onError: (Object error) => error,
            ),
      );
      final results = await Future.wait(requests);
      expect(results.whereType<Map<String, String>>(), hasLength(64));
      final rejected = results.whereType<StateError>().toList();
      expect(rejected, hasLength(16));
      expect(
        rejected.map((error) => error.message),
        everyElement('Media authentication queue is full'),
      );
      expect(await auth.headersForAsync(_mediaUrl), isNotEmpty);
    });

    test('keeps the signing identity separate across auth scopes', () async {
      final first = nostr.Keys.generate();
      final second = nostr.Keys.generate();
      for (final keys in [first, second]) {
        final header = await _auth(nsec: keys.nsec).headersForAsync(_mediaUrl);
        final encoded = header['Authorization']!.substring('Nostr '.length);
        final event = nostr.Event.fromJson(
          utf8.decode(base64Url.decode(base64Url.normalize(encoded))),
        );
        expect(event.pubkey, keys.public);
      }
    });

    test('does not expose auth to other origins or invalid paths', () async {
      final auth = _auth(nsec: nostr.Keys.generate().nsec);
      for (final url in [
        'https://elsewhere.com/media/a.png',
        '$_relayBase/media-evil/a.png',
        'ftp://relay.example.com/media/a.png',
      ]) {
        expect(await auth.headersForAsync(url), isEmpty);
      }
      expect(await _auth().headersForAsync(_mediaUrl), isEmpty);
      expect(await _auth(nsec: 'invalid').headersForAsync(_mediaUrl), isEmpty);
    });

    test(
      'a failed queued job propagates without poisoning later jobs',
      () async {
        var calls = 0;
        final auth = _auth(
          nsec: nostr.Keys.generate().nsec,
          now: () {
            if (calls++ == 0) throw StateError('clock unavailable');
            return DateTime.now();
          },
        );
        final failed = auth.headersForAsync(_mediaUrl);
        final next = auth.headersForAsync(_mediaUrl);
        await expectLater(failed, throwsStateError);
        expect(await next, isNotEmpty);
      },
    );
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
      'waits for asynchronous auth before fetching, without synchronous signing',
      () async {
        var fetches = 0;
        final auth = _DeferredMediaAuth();
        final client = http_testing.MockClient((request) async {
          fetches++;
          expect(request.headers['Authorization'], 'Nostr deferred');
          return http.Response.bytes(_pngBytes, 200);
        });
        addTearDown(client.close);
        final provider = MediaImageProvider(
          url: _mediaUrl,
          auth: auth,
          client: client,
        );
        final loaded = expectLater(
          _wait(provider.resolve(ImageConfiguration.empty)),
          completes,
        );
        await Future<void>.delayed(Duration.zero);
        expect(fetches, 0);
        auth.headers.complete({'Authorization': 'Nostr deferred'});
        await loaded;
        expect(fetches, 1);
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

class _DeferredMediaAuth extends MediaGetAuthService {
  _DeferredMediaAuth() : super(baseUrl: _relayBase, nsec: null);
  final headers = Completer<Map<String, String>>();
  @override
  Future<Map<String, String>> headersForAsync(String url) => headers.future;
  @override
  Map<String, String> headersFor(String url) =>
      throw StateError('Image layout must not sign synchronously');
}
