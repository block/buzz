import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/stickers/sticker_preview.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _author =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _hash =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _cacheUrl =
    'https://relay.example/media/sticker/$_author/cats/Wave_1/$_hash';

Widget _testable(Widget child) {
  return ProviderScope(
    overrides: [relayConfigProvider.overrideWith(_FakeRelayConfigNotifier.new)],
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
    final image = tester.widget<Image>(imageFinder);
    expect((image.image as NetworkImage).url, _cacheUrl);
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

    final image = tester.widget<Image>(
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

    final image = tester.widget<Image>(
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
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(
    baseUrl: 'https://relay.example/workspace?ignored=true',
  );
}
