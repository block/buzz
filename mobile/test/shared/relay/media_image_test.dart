import 'dart:convert';
import 'dart:async';
import 'dart:typed_data';

import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
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

    test(
      'abandoning a load another consumer still waits for keeps it cached',
      () async {
        var fetches = 0;
        var aborted = false;
        final response = Completer<void>();
        final client = _StreamingClient((request) async {
          fetches += 1;
          (request as http.AbortableRequest).abortTrigger!.then((_) {
            if (!response.isCompleted) aborted = true;
          });
          await response.future;
          return http.StreamedResponse(Stream.value(_pngBytes), 200);
        });
        final provider = MediaImageProvider(
          url: _mediaUrl,
          auth: _auth(),
          client: client,
        );
        const config = ImageConfiguration.empty;
        final cell = provider.resolve(config);
        final cellListener = ImageStreamListener((_, _) {});
        cell.addListener(cellListener);
        final prefetch = provider.resolve(config);
        final loaded = Completer<void>();
        final prefetchListener = ImageStreamListener((image, _) {
          image.dispose();
          if (!loaded.isCompleted) loaded.complete();
        });
        prefetch.addListener(prefetchListener);

        // The cell scrolls away while the prefetch still wants the image.
        cell.removeListener(cellListener);
        abandonPendingImage(cell, provider, config);
        response.complete();
        await loaded.future.timeout(const Duration(seconds: 2));
        prefetch.removeListener(prefetchListener);

        expect(aborted, isFalse);
        expect(
          PaintingBinding.instance.imageCache.containsKey(provider),
          isTrue,
        );
        await _wait(provider.resolve(config));
        expect(fetches, 1);
      },
    );

    test('abandoning a load nobody else waits for aborts it', () async {
      final aborted = Completer<void>();
      final client = _StreamingClient((request) async {
        (request as http.AbortableRequest).abortTrigger!.then((_) {
          if (!aborted.isCompleted) aborted.complete();
        });
        await aborted.future;
        throw http.RequestAbortedException(request.url);
      });
      final provider = MediaImageProvider(
        url: _mediaUrl,
        auth: _auth(),
        client: client,
      );
      final stream = provider.resolve(ImageConfiguration.empty);
      final listener = ImageStreamListener((_, _) {});
      stream.addListener(listener);
      stream.removeListener(listener);
      abandonPendingImage(stream, provider, ImageConfiguration.empty);
      await aborted.future.timeout(const Duration(seconds: 2));
      expect(
        PaintingBinding.instance.imageCache.containsKey(provider),
        isFalse,
      );
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

    test(
      'a download shared by two sizes outlives one abandoned size',
      () async {
        var fetches = 0;
        var aborts = 0;
        final response = Completer<void>();
        final client = _StreamingClient((request) async {
          fetches += 1;
          final trigger = (request as http.AbortableRequest).abortTrigger!;
          unawaited(
            trigger.then((_) {
              if (!response.isCompleted) aborts += 1;
            }),
          );
          await Future.any([response.future, trigger]);
          if (!response.isCompleted) {
            throw http.RequestAbortedException(request.url);
          }
          return http.StreamedResponse(Stream.value(_pngBytes), 200);
        });
        final auth = _auth(nsec: nostr.Keys.generate().nsec);
        final base = MediaImageProvider(
          url: _mediaUrl,
          auth: auth,
          client: client,
        );
        const config = ImageConfiguration.empty;
        final smallProvider = ResizeImage(base, width: 40);
        final largeProvider = ResizeImage(base, width: 120);
        final small = smallProvider.resolve(config);
        final smallListener = ImageStreamListener((_, _) {});
        small.addListener(smallListener);
        final largeDone = _wait(largeProvider.resolve(config));
        await pumpEventQueue();

        small.removeListener(smallListener);
        abandonPendingImage(small, smallProvider, config);
        await pumpEventQueue();
        expect(aborts, 0);
        response.complete();
        await largeDone.timeout(const Duration(seconds: 2));
        expect(fetches, 1);
        expect(aborts, 0);
        expect(MediaImageProvider.debugCooldownUrls, isEmpty);
      },
    );

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

    test(
      'a slow body that keeps arriving outlives the stall timeout',
      () async {
        const gap = Duration(milliseconds: 60);
        final client = _StreamingClient((request) async {
          // Honor the abort trigger the way the real IO client does.
          var aborted = false;
          (request as http.AbortableRequest).abortTrigger!.then((_) {
            aborted = true;
          });
          Stream<List<int>> trickle() async* {
            for (var i = 0; i < _pngBytes.length; i += 12) {
              await Future<void>.delayed(gap);
              if (aborted) throw http.RequestAbortedException(request.url);
              yield _pngBytes.sublist(i, (i + 12).clamp(0, _pngBytes.length));
            }
          }

          return http.StreamedResponse(trickle(), 200);
        });
        final stopwatch = Stopwatch()..start();
        await _wait(
          MediaImageProvider(
            url: _mediaUrl,
            auth: _auth(),
            client: client,
            stallTimeout: gap * 2.5,
          ).resolve(ImageConfiguration.empty),
        ).timeout(const Duration(seconds: 5));
        // The whole body took several stall windows; only gaps are bounded.
        expect(stopwatch.elapsed, greaterThan(gap * 5));
      },
    );

    test('a body that never ends is aborted at the fetch cap', () async {
      final client = _StreamingClient((request) async {
        var aborted = false;
        (request as http.AbortableRequest).abortTrigger!.then((_) {
          aborted = true;
        });
        Stream<List<int>> endless() async* {
          while (true) {
            await Future<void>.delayed(const Duration(milliseconds: 20));
            if (aborted) throw http.RequestAbortedException(request.url);
            yield const [0];
          }
        }

        return http.StreamedResponse(endless(), 200);
      });
      final error = await _waitError(
        MediaImageProvider(
          url: _mediaUrl,
          auth: _auth(),
          client: client,
          stallTimeout: const Duration(seconds: 5),
          maxFetchDuration: const Duration(milliseconds: 200),
        ).resolve(ImageConfiguration.empty),
      ).timeout(const Duration(seconds: 3));
      expect(error, isA<http.RequestAbortedException>());
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

    test(
      'honors Retry-After on 429 (capped) without reading the body',
      () async {
        var fetches = 0;
        // A body that never ends: the error must not wait for it.
        Stream<List<int>> endless() async* {
          while (true) {
            await Future<void>.delayed(const Duration(milliseconds: 20));
            yield const [0];
          }
        }

        final client = _StreamingClient((request) async {
          fetches += 1;
          return http.StreamedResponse(
            endless(),
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

        final error = await attempt().timeout(const Duration(seconds: 3));
        expect(error, isA<NetworkImageLoadException>());
        expect(fetches, 1);

        current = current.add(const Duration(seconds: 60));
        expect(await attempt(), isA<MediaImageCooldownException>());
        expect(fetches, 1);

        current = current.add(const Duration(seconds: 61));
        await attempt();
        expect(fetches, 2);
      },
    );
  });

  group('stalls', () {
    // Each row stalls at a different point; both must abort at the stall
    // timeout, well before the fetch cap, and put the URL on cooldown.
    final rows =
        <
          String,
          Future<http.StreamedResponse> Function(
            http.BaseRequest request,
            Future<void> aborted,
          )
        >{
          'headers never arrive': (request, aborted) async {
            await aborted;
            throw http.RequestAbortedException(request.url);
          },
          'the body stops partway': (request, aborted) async {
            Stream<List<int>> body() async* {
              yield _pngBytes.sublist(0, 8);
              await aborted;
              throw http.RequestAbortedException(request.url);
            }

            return http.StreamedResponse(body(), 200);
          },
        };
    for (final MapEntry(key: name, value: respond) in rows.entries) {
      test('a silent connection is aborted when $name', () async {
        final client = _StreamingClient(
          (request) => respond(
            request,
            (request as http.AbortableRequest).abortTrigger!,
          ),
        );
        final stopwatch = Stopwatch()..start();
        final error = await _waitError(
          MediaImageProvider(
            url: _mediaUrl,
            auth: _auth(),
            client: client,
            stallTimeout: const Duration(milliseconds: 150),
            maxFetchDuration: const Duration(seconds: 30),
          ).resolve(ImageConfiguration.empty),
        ).timeout(const Duration(seconds: 5));
        expect(error, isA<http.RequestAbortedException>());
        expect(stopwatch.elapsed, lessThan(const Duration(seconds: 5)));
        expect(MediaImageProvider.debugCooldownUrls, [_mediaUrl]);
      });
    }
  });

  testWidgets('an image that leaves before it loads stops downloading, '
      'unless another widget still shows it', (tester) async {
    var aborted = false;
    final client = _StreamingClient((request) async {
      // A slow link: the body has started but does not finish in this test.
      final body = StreamController<List<int>>()..add(const [0]);
      (request as http.AbortableRequest).abortTrigger!.then((_) {
        aborted = true;
        body
          ..addError(http.RequestAbortedException(request.url))
          ..close();
      });
      return http.StreamedResponse(body.stream, 200);
    });
    final shown = ValueNotifier(2);
    addTearDown(shown.dispose);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          mediaGetAuthServiceProvider.overrideWithValue(_auth()),
          mediaHttpClientProvider.overrideWithValue(client),
        ],
        child: MaterialApp(
          home: ValueListenableBuilder<int>(
            valueListenable: shown,
            builder: (_, count, _) => Column(
              children: [
                for (var i = 0; i < count; i++)
                  const SizedBox.square(
                    dimension: 40,
                    child: MediaImage(url: _mediaUrl),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    Future<void> settle() async {
      for (var i = 0; i < 3; i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
        await tester.pump();
      }
    }

    await settle();
    // One copy scrolls away; the other still shows the image.
    shown.value = 1;
    await settle();
    expect(aborted, isFalse);

    // The last one leaves: the download stops, and the URL itself did not
    // fail, so it is not put on cooldown.
    shown.value = 0;
    await settle();
    expect(aborted, isTrue);
    expect(MediaImageProvider.debugCooldownUrls, isEmpty);
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
