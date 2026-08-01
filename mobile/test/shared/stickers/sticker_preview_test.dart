import 'dart:convert';

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/stickers/sticker_preview.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

const _author =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _hash =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _cacheUrl =
    'https://relay.example/media/sticker/$_author/cats/Wave_1/$_hash';

/// Minimal valid 1x1 transparent PNG.
final _pngBytes = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAA'
  'AAYAAjCB0C8AAAAASUVORK5CYII=',
);

Widget _testable(
  Widget child, {
  bool signed = false,
  List<Override> overrides = const [],
}) {
  return ProviderScope(
    overrides: [
      relayConfigProvider.overrideWith(
        signed ? _SignedRelayConfigNotifier.new : _FakeRelayConfigNotifier.new,
      ),
      ...overrides,
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(body: child),
    ),
  );
}

String _renderedText(WidgetTester tester) => tester
    .widgetList<RichText>(find.byType(RichText))
    .map((text) => text.text.toPlainText())
    .join('\n');

void main() {
  setUp(() {
    // Earlier cases in this file let real fetches fail against the fake relay
    // host, which arms the shared failure cooldown for `_cacheUrl`.
    MediaImageProvider.debugResetCooldowns();
    PaintingBinding.instance.imageCache.clear();
    PaintingBinding.instance.imageCache.clearLiveImages();
  });

  testWidgets('renders a valid sticker through the active relay cache', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: ':Wave_1:',
          tags: [
            ['sticker', '30031:$_author:cats', 'Wave_1', _hash],
          ],
        ),
      ),
    );

    final imageFinder = find.byKey(
      const ValueKey('message-sticker-image:$_cacheUrl'),
    );
    expect(imageFinder, findsOneWidget);
    final image = tester.widget<MediaImage>(imageFinder);
    expect(image.url, _cacheUrl);
    expect(image.width, stickerPreviewSize);
    expect(image.height, stickerPreviewSize);
    expect(_renderedText(tester), isNot(contains(':Wave_1:')));
  });

  testWidgets('shows unavailable state for invalid tags and keeps fallback', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: ':wave:',
          tags: [
            ['sticker', '30031:$_author:cats', 'wave', _hash],
            ['sticker', '30031:$_author:cats', 'other', _hash],
          ],
        ),
      ),
    );

    expect(
      find.byKey(const ValueKey('message-sticker-unavailable')),
      findsOneWidget,
    );
    expect(find.text('Sticker unavailable'), findsOneWidget);
    expect(_renderedText(tester), contains(':wave:'));
    expect(find.byType(Image), findsNothing);
  });

  testWidgets('uses a compact sticker in truncated message previews', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: ':Wave_1:',
          maxLines: 1,
          tags: [
            ['sticker', '30031:$_author:cats', 'Wave_1', _hash],
          ],
        ),
      ),
    );

    final image = tester.widget<MediaImage>(
      find.byKey(const ValueKey('message-sticker-image:$_cacheUrl')),
    );
    expect(image.width, compactStickerPreviewSize);
    expect(image.height, compactStickerPreviewSize);
  });

  testWidgets('network image errors use the deterministic unavailable state', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: ':Wave_1:',
          tags: [
            ['sticker', '30031:$_author:cats', 'Wave_1', _hash],
          ],
        ),
      ),
    );

    final image = tester.widget<MediaImage>(
      find.byKey(const ValueKey('message-sticker-image:$_cacheUrl')),
    );
    final errorBuilder = image.errorBuilder;
    expect(errorBuilder, isNotNull);

    await tester.pumpWidget(
      _testable(
        Builder(
          builder: (context) => errorBuilder!(
            context,
            Exception('failed image'),
            StackTrace.empty,
          ),
        ),
      ),
    );

    expect(find.text('Sticker unavailable'), findsOneWidget);
    expect(
      find.byKey(const ValueKey('message-sticker-unavailable')),
      findsOneWidget,
    );
  });

  testWidgets('sticker reads are signed with Blossom get-auth headers', (
    tester,
  ) async {
    // Regression guard: an unsigned `Image.network` fetch renders every valid
    // sticker as unavailable when the relay runs with
    // BUZZ_REQUIRE_MEDIA_GET_AUTH=true.
    final requestedUrls = <String>[];
    final capturedHeaders = <String, String>{};
    final client = http_testing.MockClient((request) async {
      requestedUrls.add(request.url.toString());
      capturedHeaders.addAll(request.headers);
      return http.Response.bytes(
        _pngBytes,
        200,
        headers: const {'content-type': 'image/png'},
      );
    });
    addTearDown(client.close);

    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: ':Wave_1:',
          tags: [
            ['sticker', '30031:$_author:cats', 'Wave_1', _hash],
          ],
        ),
        signed: true,
        overrides: [mediaHttpClientProvider.overrideWithValue(client)],
      ),
    );
    await tester.pump();

    expect(requestedUrls, contains(_cacheUrl));
    expect(capturedHeaders['Authorization'], startsWith('Nostr '));
    expect(
      find.byKey(const ValueKey('message-sticker-unavailable')),
      findsNothing,
    );
  });

  testWidgets('sticker sizing and fit survive the authenticated image path', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: ':Wave_1:',
          tags: [
            ['sticker', '30031:$_author:cats', 'Wave_1', _hash],
          ],
        ),
      ),
    );

    final image = tester.widget<MediaImage>(
      find.byKey(const ValueKey('message-sticker-image:$_cacheUrl')),
    );
    expect(image.fit, BoxFit.contain);
    expect(image.decodeWidth, stickerPreviewSize);
    expect(image.semanticLabel, 'Sticker :Wave_1:');
  });
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(
    baseUrl: 'https://relay.example/workspace?ignored=true',
  );
}

/// Relay config with signing key material, so media reads can be authenticated.
class _SignedRelayConfigNotifier extends RelayConfigNotifier {
  static final _nsec = nostr.Keys.generate().nsec;

  @override
  RelayConfig build() => RelayConfig(
    baseUrl: 'https://relay.example/workspace?ignored=true',
    nsec: _nsec,
  );
}
