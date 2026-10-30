import 'dart:async';

import 'package:buzz/features/channels/emoji_picker.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

NostrEvent _entry(String name, {int createdAt = 1}) => NostrEvent(
  id: 'id-$name-$createdAt',
  pubkey: 'author',
  createdAt: createdAt,
  kind: EventKind.emojiEntry,
  tags: [
    ['d', name],
    ['emoji', name, 'https://cdn.test/$name.png'],
  ],
  content: name.replaceAll(RegExp(r'[_-]'), ' '),
  sig: '',
);

class _FakeSession extends RelaySessionNotifier {
  final searches =
      <({NostrFilter filter, Completer<List<NostrEvent>> reply})>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [
    NostrEvent(
      id: 'legacy-set',
      pubkey: 'author',
      createdAt: 1,
      kind: kindEmojiSet,
      tags: [
        ['d', customEmojiSetDTag],
        ['emoji', 'bee', 'https://cdn.test/bee.png'],
      ],
      content: '',
      sig: '',
    ),
  ];

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    final reply = Completer<List<NostrEvent>>();
    searches.add((filter: filters.single, reply: reply));
    return reply.future;
  }
}

Finder _catalogTile(String name) =>
    find.byKey(ValueKey('emoji-tile-catalog-$name'));

Future<void> _type(WidgetTester tester, String text) async {
  await tester.enterText(
    find.byKey(const ValueKey('emoji-picker-search')),
    text,
  );
  await tester.pump(const Duration(milliseconds: 299));
}

void main() {
  testWidgets('searching the catalog from the picker', (tester) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final session = _FakeSession();
    final selected = <String>[];
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
        child: EmojiPickerSheet(onSelect: selected.add),
      ),
    );
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(EmojiPickerSheet)),
    );

    // One character is too short, and typing waits out the debounce.
    await _type(tester, 'p');
    await tester.pump(const Duration(milliseconds: 100));
    expect(session.searches, isEmpty);
    await _type(tester, 'pa');
    expect(session.searches, isEmpty);
    await tester.pump(const Duration(milliseconds: 2));
    expect(session.searches, hasLength(1));
    expect(session.searches.single.filter.toJson(), {
      'kinds': [30180],
      'search': 'pa',
      'search_mode': 'prefix',
      'limit': 50,
    });

    // The query changes before "pa" answers. Only the newest answer shows.
    await _type(tester, 'party_par');
    await tester.pump(const Duration(milliseconds: 2));
    expect(session.searches, hasLength(2));
    expect(session.searches.last.filter.search, 'party par');
    session.searches.last.reply.complete([_entry('party_parrot')]);
    await tester.pump();
    expect(_catalogTile('party_parrot'), findsOneWidget);

    session.searches.first.reply.complete([_entry('pancake')]);
    await tester.pump();
    expect(_catalogTile('pancake'), findsNothing);
    expect(_catalogTile('party_parrot'), findsOneWidget);
    expect(find.bySemanticsLabel(':party_parrot:'), findsOneWidget);
    expect(find.bySemanticsLabel('Search emoji'), findsOneWidget);

    // Picking a result emits the shortcode, and the message or reaction that
    // follows carries the result's URL.
    await tester.tap(_catalogTile('party_parrot'));
    await tester.pump();
    expect(selected, [':party_parrot:']);
    final palette = container.read(customEmojiListProvider);
    expect(buildCustomEmojiTags('hi :party_parrot:', palette), [
      ['emoji', 'party_parrot', 'https://cdn.test/party_parrot.png'],
    ]);
    expect(
      reactionEmojiUrl(':party_parrot:', palette),
      'https://cdn.test/party_parrot.png',
    );

    // A failed search keeps the palette and the earlier results, and Retry
    // asks again.
    await _type(tester, 'bee');
    await tester.pump(const Duration(milliseconds: 2));
    session.searches.last.reply.completeError(Exception('relay down'));
    await tester.pump();
    expect(find.text('Couldn’t search the emoji catalog.'), findsOneWidget);
    expect(find.byKey(const ValueKey('emoji-tile-custom-bee')), findsOneWidget);
    expect(find.bySemanticsLabel('Retry'), findsOneWidget);

    final before = session.searches.length;
    await tester.tap(find.byKey(const ValueKey('emoji-catalog-retry')));
    await tester.pump();
    expect(session.searches, hasLength(before + 1));
    expect(session.searches.last.filter.search, 'bee');
    session.searches.last.reply.complete([_entry('beehive')]);
    await tester.pump();
    expect(find.text('Couldn’t search the emoji catalog.'), findsNothing);
    expect(_catalogTile('beehive'), findsOneWidget);
    expect(find.byKey(const ValueKey('emoji-tile-custom-bee')), findsOneWidget);
  });
}
