import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:buzz/shared/community/community_icon_cache.dart';
import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  const relay = 'wss://relay.example.com';
  const key = 'https://relay.example.com/';
  final artwork = emojiAvatarDataUrl('🐝', 0xFFFFCC00);
  late SharedPreferences prefs;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
  });

  ProviderContainer containerFor(
    Future<http.Response> Function(http.Request) handler,
  ) {
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        communityIconHttpClientProvider.overrideWithValue(MockClient(handler)),
      ],
    );
    addTearDown(container.dispose);
    return container;
  }

  test('fetches and saves inline NIP-11 artwork using public HTTP', () async {
    final container = containerFor((request) async {
      expect(request.url.toString(), key);
      expect(request.headers['Accept'], 'application/nostr+json');
      expect(request.headers.containsKey('authorization'), isFalse);
      return http.Response(jsonEncode({'icon': artwork}), 200);
    });
    expect(await container.read(communityIconProvider(relay).future), artwork);
    expect(container.read(communityIconCacheProvider)[key], artwork);
    expect(prefs.getString('buzz.community-icons.v3'), contains('image/svg'));
  });

  test(
    'saved artwork is synchronous after reopening and an offline restart',
    () async {
      final first = containerFor(
        (_) async => http.Response(jsonEncode({'icon': artwork}), 200),
      );
      final presentation = communityIconPresentationProvider(relay);
      final listener = first.listen(presentation, (_, _) {});
      await first.read(communityIconProvider(relay).future);
      listener.close();
      await first.pump();
      first.invalidate(communityIconProvider);
      expect(first.read(presentation), artwork);

      final response = Completer<http.Response>();
      final restarted = containerFor((_) => response.future);
      final subscription = restarted.listen(presentation, (_, _) {});
      addTearDown(subscription.close);
      expect(restarted.read(presentation), artwork);
      expect(
        restarted.read(
          communityIconPresentationProvider('wss://other.example.com'),
        ),
        isNull,
      );
      response.complete(http.Response('offline', 503));
      await restarted.read(communityIconProvider(relay).future);
      expect(restarted.read(presentation), artwork);
    },
  );

  test(
    'downloads remote image bytes and retains them when refresh fails',
    () async {
      var offline = false;
      final png = base64Decode(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4nGNgAAAAAgABSK+kcQAAAABJRU5ErkJggg==',
      );
      final container = containerFor((request) async {
        if (offline) return http.Response('offline', 503);
        if (request.url.path == '/icon.png') {
          expect(request.headers.containsKey('authorization'), isFalse);
          return http.Response.bytes(
            png,
            200,
            headers: {'content-type': 'image/png'},
          );
        }
        return http.Response(
          '{"icon":"https://relay.example.com/icon.png"}',
          200,
        );
      });
      final provider = communityIconProvider(relay);
      final subscription = container.listen(provider, (_, _) {});
      addTearDown(subscription.close);
      final saved = await container.read(provider.future);
      expect(saved, isNotNull);
      final dimensions = await _dimensions(
        UriData.parse(saved!).contentAsBytes(),
      );
      expect(dimensions, (1, 1));
      offline = true;
      container.invalidate(provider);
      expect(await container.read(provider.future), saved);
      final restarted = containerFor(
        (_) async => http.Response('offline', 503),
      );
      expect(restarted.read(communityIconPresentationProvider(relay)), saved);
    },
  );

  test(
    'refresh replaces changed artwork and clears explicitly removed icons',
    () async {
      var document = jsonEncode({'icon': artwork});
      final container = containerFor((_) async => http.Response(document, 200));
      final provider = communityIconProvider(relay);
      final presentation = communityIconPresentationProvider(relay);
      final subscription = container.listen(presentation, (_, _) {});
      addTearDown(subscription.close);
      await container.read(provider.future);
      final newArtwork = emojiAvatarDataUrl('🥳', 0xFFFF8652);
      document = jsonEncode({'icon': newArtwork});
      container.invalidate(provider);
      expect(container.read(presentation), artwork);
      await container.read(provider.future);
      expect(container.read(presentation), newArtwork);
      document = '{"icon":""}';
      container.invalidate(provider);
      await container.read(provider.future);
      expect(container.read(presentation), isNull);
      expect(container.read(communityIconCacheProvider), isEmpty);
    },
  );

  test('an invalidated older lookup cannot overwrite newer artwork', () async {
    final slow = Completer<http.Response>();
    var calls = 0;
    final container = containerFor(
      (_) => ++calls == 1
          ? slow.future
          : Future.value(http.Response(jsonEncode({'icon': artwork}), 200)),
    );
    final provider = communityIconProvider(relay);
    final subscription = container.listen(provider, (_, _) {});
    addTearDown(subscription.close);
    final oldLookup = container.read(provider.future);
    container.invalidate(provider);
    await container.read(provider.future);
    slow.complete(http.Response('{"icon":""}', 200));
    await oldLookup;
    expect(container.read(communityIconCacheProvider)[key], artwork);
  });

  test('ignores corrupt persistence and bounds cache entries', () async {
    await prefs.setString('buzz.community-icons.v3', '{bad json');
    final container = containerFor((_) async => http.Response('{}', 200));
    expect(container.read(communityIconCacheProvider), isEmpty);
    final cache = container.read(communityIconCacheProvider.notifier);
    for (var i = 0; i < 35; i++) {
      await cache.remember('https://relay$i.example.com/', artwork);
    }
    expect(container.read(communityIconCacheProvider).length, 32);
    expect(
      container
          .read(communityIconCacheProvider)
          .containsKey('https://relay0.example.com/'),
      isFalse,
    );
  });

  for (final saved in [false, true]) {
    for (final rejection in ['oversized', 'invalid MIME', 'HTTP failure']) {
      test('$rejection never exposes an unchecked URL, saved=$saved', () async {
        final container = containerFor((request) async {
          if (request.url.path == '/') {
            return http.Response(
              '{"icon":"https://relay.example.com/rejected.png"}',
              200,
            );
          }
          return http.Response.bytes(
            List.filled(rejection == 'oversized' ? 256 * 1024 + 1 : 4, 0),
            rejection == 'HTTP failure' ? 503 : 200,
            headers: {
              'content-type': rejection == 'invalid MIME'
                  ? 'text/html'
                  : 'image/png',
            },
          );
        });
        if (saved) {
          await container
              .read(communityIconCacheProvider.notifier)
              .remember(key, artwork);
        }
        final presentation = communityIconPresentationProvider(relay);
        final subscription = container.listen(presentation, (_, _) {});
        addTearDown(subscription.close);
        final expected = saved ? artwork : null;
        expect(
          await container.read(communityIconProvider(relay).future),
          expected,
        );
        expect(container.read(presentation), expected);
        expect(container.read(communityIconCacheProvider)[key], expected);
      });
    }
  }

  for (final stalledImage in [false, true]) {
    test('timeout aborts the request, image=$stalledImage', () async {
      var aborted = false;
      var pending = false;
      final started = Completer<void>();
      final client = MockClient.streaming((request, _) async {
        if (stalledImage && request.url.path == '/') {
          return http.StreamedResponse(
            Stream.value(
              utf8.encode('{"icon":"https://relay.example.com/stalled.png"}'),
            ),
            200,
          );
        }
        expect(request, isA<http.AbortableRequest>());
        pending = true;
        started.complete();
        await (request as http.AbortableRequest).abortTrigger;
        pending = false;
        aborted = true;
        throw http.RequestAbortedException(request.url);
      });
      final container = ProviderContainer(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          communityIconHttpClientProvider.overrideWithValue(client),
        ],
      );
      addTearDown(container.dispose);
      addTearDown(client.close);
      final subscription = container.listen(
        communityIconProvider(relay),
        (_, _) {},
      );
      addTearDown(subscription.close);
      final result = container.read(communityIconProvider(relay).future);
      await started.future;
      expect(pending, isTrue);
      expect(await result, isNull);
      expect(aborted, isTrue);
      expect(pending, isFalse);
    });
  }

  for (final inline in [false, true]) {
    test('rejects compressed oversized dimensions, inline=$inline', () async {
      final bytes = await File(
        'test/fixtures/community_icons/compressed-16000.png',
      ).readAsBytes();
      expect(bytes.length, lessThan(256 * 1024));
      // Read the encoded descriptor, never allocate this 256-million-pixel image.
      expect(await _dimensions(bytes), (16000, 16000));
      final source = Uri.dataFromBytes(bytes, mimeType: 'image/png').toString();
      final container = containerFor((request) async {
        if (request.url.path == '/') {
          return http.Response(
            jsonEncode({
              'icon': inline ? source : 'https://relay.example.com/icon.png',
            }),
            200,
          );
        }
        return http.Response.bytes(
          bytes,
          200,
          headers: {'content-type': 'image/png'},
        );
      });
      expect(await container.read(communityIconProvider(relay).future), isNull);
      expect(container.read(communityIconCacheProvider), isEmpty);
      expect(prefs.getString('buzz.community-icons.v3'), isNull);
      final restarted = containerFor(
        (_) async => http.Response('offline', 503),
      );
      expect(restarted.read(communityIconPresentationProvider(relay)), isNull);
    });
  }

  test(
    'old cache cannot reintroduce oversized artwork after restart',
    () async {
      final bytes = await File(
        'test/fixtures/community_icons/compressed-16000.png',
      ).readAsBytes();
      await prefs.setString(
        'buzz.community-icons.v1',
        jsonEncode({
          key: Uri.dataFromBytes(bytes, mimeType: 'image/png').toString(),
        }),
      );
      final container = containerFor(
        (_) async => http.Response('offline', 503),
      );
      expect(container.read(communityIconPresentationProvider(relay)), isNull);
      expect(container.read(communityIconCacheProvider), isEmpty);
    },
  );

  for (final remote in [false, true]) {
    test('rejects SVG-wrapped oversized raster, remote=$remote', () async {
      final bytes = await File(
        'test/fixtures/community_icons/compressed-8192.png',
      ).readAsBytes();
      expect(await _dimensions(bytes), (8192, 8192));
      final svg =
          '<svg xmlns="http://www.w3.org/2000/svg">'
          '<image href="data:image/png;base64,${base64Encode(bytes)}"/>'
          '</svg>';
      expect(utf8.encode(svg).length, lessThan(256 * 1024));
      final wrapped = Uri.dataFromString(
        svg,
        mimeType: 'image/svg+xml',
        encoding: utf8,
      ).toString();
      final container = containerFor((request) async {
        if (request.url.path == '/icon.svg') {
          return http.Response(
            svg,
            200,
            headers: {'content-type': 'image/svg+xml'},
          );
        }
        return http.Response(
          jsonEncode({'icon': remote ? '${key}icon.svg' : wrapped}),
          200,
        );
      });
      expect(await container.read(communityIconProvider(relay).future), isNull);
      expect(container.read(communityIconCacheProvider), isEmpty);
      expect(prefs.getString('buzz.community-icons.v3'), isNull);

      // Even an entry persisted by the previous version must not render.
      await prefs.setString(
        'buzz.community-icons.v2',
        jsonEncode({key: wrapped}),
      );
      final restarted = containerFor(
        (_) async => http.Response('offline', 503),
      );
      expect(restarted.read(communityIconPresentationProvider(relay)), isNull);
      expect(restarted.read(communityIconCacheProvider), isEmpty);
    });
  }

  for (final radius in ['', ' rx="112"', ' rx="256"']) {
    test('retains desktop emoji artwork with radius $radius', () async {
      final svg = UriData.parse(
        artwork,
      ).contentAsString(encoding: utf8).replaceFirst(' rx="256"', radius);
      final container = containerFor(
        (_) async => http.Response(
          jsonEncode({
            'icon': Uri.dataFromString(
              svg,
              mimeType: 'image/svg+xml',
              encoding: utf8,
            ).toString(),
          }),
          200,
        ),
      );
      expect(
        await container.read(communityIconProvider(relay).future),
        artwork,
      );
    });
  }

  for (final extra in [
    '<image href="https://relay.example.com/huge.png"/>',
    '<image href="data:image/png;base64,AAAA"/>',
    '<style>text { fill: url(https://relay.example.com/resource); }</style>',
  ]) {
    test('rejects resource added to otherwise valid emoji: $extra', () async {
      final svg = UriData.parse(
        artwork,
      ).contentAsString(encoding: utf8).replaceFirst('</svg>', '$extra</svg>');
      final container = containerFor(
        (_) async => http.Response(
          jsonEncode({
            'icon': Uri.dataFromString(
              svg,
              mimeType: 'image/svg+xml',
              encoding: utf8,
            ).toString(),
          }),
          200,
        ),
      );
      await container
          .read(communityIconCacheProvider.notifier)
          .remember(key, artwork);
      expect(
        await container.read(communityIconProvider(relay).future),
        artwork,
      );
      expect(container.read(communityIconCacheProvider)[key], artwork);
    });
  }

  test(
    'cached raster is bounded before rendering and after offline restart',
    () async {
      final bytes = await File(
        'test/fixtures/community_icons/landscape-1024.png',
      ).readAsBytes();
      expect(await _dimensions(bytes), (1024, 512));
      final container = containerFor(
        (_) async => http.Response(
          jsonEncode({
            'icon': Uri.dataFromBytes(bytes, mimeType: 'image/png').toString(),
          }),
          200,
        ),
      );
      final subscription = container.listen(
        communityIconProvider(relay),
        (_, _) {},
      );
      addTearDown(subscription.close);
      final saved = await container.read(communityIconProvider(relay).future);
      expect(saved, isNotNull);
      expect(await _dimensions(UriData.parse(saved!).contentAsBytes()), (
        512,
        256,
      ));
      final restarted = containerFor(
        (_) async => http.Response('offline', 503),
      );
      final restored = restarted.read(communityIconPresentationProvider(relay));
      expect(restored, saved);
      expect(await _dimensions(UriData.parse(restored!).contentAsBytes()), (
        512,
        256,
      ));
    },
  );

  for (final stalledImage in [false, true]) {
    test(
      'invalidation and disposal abort pending headers, image=$stalledImage',
      () async {
        var started = 0;
        var active = 0;
        var aborted = 0;
        final client = MockClient.streaming((request, _) async {
          if (stalledImage && request.url.path == '/') {
            return http.StreamedResponse(
              Stream.value(
                utf8.encode('{"icon":"https://relay.example.com/stalled.png"}'),
              ),
              200,
            );
          }
          started++;
          active++;
          await (request as http.AbortableRequest).abortTrigger;
          active--;
          aborted++;
          throw http.RequestAbortedException(request.url);
        });
        final container = ProviderContainer(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            communityIconHttpClientProvider.overrideWithValue(client),
          ],
        );
        addTearDown(container.dispose);
        addTearDown(client.close);
        final provider = communityIconProvider(relay);
        final subscription = container.listen(provider, (_, _) {});
        for (var iteration = 0; iteration < 3; iteration++) {
          await _waitFor(() => started == iteration + 1);
          expect(active, 1);
          container.invalidate(provider);
          await _waitFor(() => aborted == iteration + 1);
        }
        await _waitFor(() => started == 4);
        expect(active, 1);
        subscription.close();
        await container.pump();
        await _waitFor(() => active == 0);
        expect(aborted, 4);
      },
    );
  }

  test('oversized downloads do not replace a usable saved image', () async {
    final container = containerFor(
      (request) async => request.url.path == '/'
          ? http.Response('{"icon":"https://relay.example.com/large.png"}', 200)
          : http.Response.bytes(
              List.filled(256 * 1024 + 1, 0),
              200,
              headers: {'content-type': 'image/png'},
            ),
    );
    await container
        .read(communityIconCacheProvider.notifier)
        .remember(key, artwork);
    expect(await container.read(communityIconProvider(relay).future), artwork);
  });
}

Future<(int, int)> _dimensions(Uint8List bytes) async {
  final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
  try {
    final descriptor = await ui.ImageDescriptor.encoded(buffer);
    try {
      return (descriptor.width, descriptor.height);
    } finally {
      descriptor.dispose();
    }
  } finally {
    buffer.dispose();
  }
}

Future<void> _waitFor(bool Function() ready) async {
  for (var i = 0; i < 100 && !ready(); i++) {
    await Future<void>.delayed(Duration.zero);
  }
  expect(ready(), isTrue, reason: 'Request cancellation did not settle');
}
