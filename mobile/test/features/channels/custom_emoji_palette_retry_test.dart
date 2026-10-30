import 'package:buzz/features/channels/emoji_picker.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

NostrEvent _set(String pubkey, String shortcode) => NostrEvent(
  id: 'id-$pubkey',
  pubkey: pubkey,
  createdAt: 1,
  kind: kindEmojiSet,
  tags: [
    ['d', customEmojiSetDTag],
    ['emoji', shortcode, 'https://example.test/$shortcode.png'],
  ],
  content: '',
  sig: '',
);

class _ScriptedSession extends RelaySessionNotifier {
  _ScriptedSession(this.responses);

  final List<Object> responses;
  int fetches = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final next = responses[fetches++];
    if (next is Exception) throw next;
    return next as List<NostrEvent>;
  }
}

Finder _tile(String shortcode) =>
    find.byKey(ValueKey('emoji-tile-custom-$shortcode'));

void main() {
  testWidgets(
    'a failed palette refresh keeps the last palette and offers retry',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final session = _ScriptedSession([
        [_set('a', 'oldparrot')],
        Exception('relay down'),
        Exception('relay still down'),
        [_set('a', 'oldparrot'), _set('b', 'newparrot')],
      ]);
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            myPubkeyProvider.overrideWithValue('self'),
            relaySessionProvider.overrideWith(() => session),
            emojiDatasetOrEmptyProvider.overrideWithValue(
              const EmojiDataset(
                categories: [],
                all: [],
                nativeToShortcode: {},
              ),
            ),
          ],
          child: EmojiPickerSheet(onSelect: (_) {}),
        ),
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(EmojiPickerSheet)),
      );

      expect(_tile('oldparrot'), findsOneWidget);
      expect(find.text('Retry'), findsNothing);

      await container.read(customEmojiPaletteProvider.notifier).refresh();
      await tester.pumpAndSettle();
      expect(_tile('oldparrot'), findsOneWidget);
      expect(find.text('Retry'), findsOneWidget);

      container.invalidate(customEmojiPaletteProvider);
      await tester.pumpAndSettle();
      expect(_tile('oldparrot'), findsOneWidget);
      expect(find.text('Retry'), findsOneWidget);

      expect(find.bySemanticsLabel('Retry'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('custom-emoji-retry')));
      await tester.pumpAndSettle();

      expect(_tile('oldparrot'), findsOneWidget);
      expect(_tile('newparrot'), findsOneWidget);
      expect(find.text('Retry'), findsNothing);
      expect(session.fetches, 4);
    },
  );

  testWidgets('a first fetch that fails shows retry, not an empty palette', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final session = _ScriptedSession([
      Exception('relay down'),
      [_set('a', 'parrot')],
      [_set('a', 'parrot')],
    ]);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          myPubkeyProvider.overrideWithValue('self'),
          relaySessionProvider.overrideWith(() => session),
          emojiDatasetOrEmptyProvider.overrideWithValue(
            const EmojiDataset(categories: [], all: [], nativeToShortcode: {}),
          ),
        ],
        child: EmojiPickerSheet(onSelect: (_) {}),
      ),
    );
    await tester.pump();

    expect(find.text('Retry'), findsOneWidget);
    expect(session.fetches, 1);
    await tester.tap(find.byKey(const ValueKey('custom-emoji-retry')));
    await tester.pumpAndSettle();
    expect(_tile('parrot'), findsOneWidget);
    expect(find.text('Retry'), findsNothing);
  });
}
