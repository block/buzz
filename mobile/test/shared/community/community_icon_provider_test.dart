import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/community/community_icon_cache.dart';
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
  final artwork = Uri.dataFromString(
    '<svg xmlns="http://www.w3.org/2000/svg"><text>🐝</text></svg>',
    mimeType: 'image/svg+xml',
    encoding: utf8,
  ).toString();
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
    expect(prefs.getString('buzz.community-icons.v1'), contains('image/svg'));
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
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==',
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
      final saved = await container.read(provider.future);
      expect(UriData.parse(saved!).contentAsBytes(), png);
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
      final newArtwork = Uri.dataFromString(
        '<svg/>',
        mimeType: 'image/svg+xml',
      ).toString();
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
    await prefs.setString('buzz.community-icons.v1', '{bad json');
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
