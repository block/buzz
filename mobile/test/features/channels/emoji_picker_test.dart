import 'package:buzz/features/channels/emoji_picker.dart';
import 'package:buzz/features/channels/recent_emoji_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

EmojiEntry _entry(
  String id, {
  required String native,
  required String categoryId,
  String? name,
  List<String> keywords = const [],
  int skinIndex = 0,
}) => EmojiEntry(
  id: id,
  name: name ?? id,
  keywords: keywords,
  native: native,
  categoryId: categoryId,
  skinIndex: skinIndex,
);

/// A miniature stand-in for the generated asset — two categories so the rail
/// has something to switch between. `emoji_data_test.dart` covers the real one.
final _dataset = () {
  final people = [
    _entry('grinning', native: '\u{1F600}', categoryId: 'people'),
    _entry(
      'point_up',
      native: '\u{261D}\u{FE0F}',
      categoryId: 'people',
      name: 'Index Pointing Up',
    ),
    _entry(
      'point_up',
      native: '\u{261D}\u{1F3FD}',
      categoryId: 'people',
      name: 'Index Pointing Up',
      skinIndex: 1,
    ),
  ];
  final nature = [
    _entry(
      'fire',
      native: '\u{1F525}',
      categoryId: 'nature',
      name: 'Fire',
      keywords: const ['flame'],
    ),
  ];
  final all = [...people, ...nature];
  return EmojiDataset(
    categories: [
      EmojiCategory(id: 'people', emoji: people),
      EmojiCategory(id: 'nature', emoji: nature),
    ],
    all: all,
    nativeToShortcode: {for (final entry in all) entry.native: ':${entry.id}:'},
  );
}();

/// A dataset tall enough that the grid actually scrolls, so rail navigation has
/// somewhere to go. [_dataset] fits on one screen and clamps to offset 0.
final _tallDataset = () {
  final people = [
    for (var i = 0; i < 200; i++)
      _entry('people_$i', native: '\u{1F600}', categoryId: 'people'),
  ];
  // Nature is tall too, so the People target isn't clamped by the end of the
  // list — this test is about landing on a header, not about the clamp.
  final nature = [
    _entry('fire', native: '\u{1F525}', categoryId: 'nature'),
    for (var i = 0; i < 200; i++)
      _entry('nature_$i', native: '\u{1F33F}', categoryId: 'nature'),
  ];
  final all = [...people, ...nature];
  return EmojiDataset(
    categories: [
      EmojiCategory(id: 'people', emoji: people),
      EmojiCategory(id: 'nature', emoji: nature),
    ],
    all: all,
    nativeToShortcode: {for (final entry in all) entry.native: ':${entry.id}:'},
  );
}();

const _customEmoji = [
  CustomEmoji(shortcode: 'partyparrot', url: 'https://example.test/parrot.gif'),
];

Future<SharedPreferences> _prefs() {
  SharedPreferences.setMockInitialValues({});
  return SharedPreferences.getInstance();
}

Future<List<String>> _pumpPicker(
  WidgetTester tester, {
  required SharedPreferences prefs,
  List<CustomEmoji> customEmoji = _customEmoji,
  EmojiDataset? dataset,
}) async {
  final selected = <String>[];
  await tester.pumpWidget(
    WidgetHelpers.testable(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        myPubkeyProvider.overrideWithValue('self'),
        emojiDatasetOrEmptyProvider.overrideWithValue(dataset ?? _dataset),
        customEmojiListProvider.overrideWithValue(customEmoji),
      ],
      child: EmojiPickerSheet(onSelect: selected.add),
    ),
  );
  await tester.pumpAndSettle();
  return selected;
}

void main() {
  group('EmojiPickerSheet', () {
    testWidgets('with no history there is no Frequently used section', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs());

      // An empty section in a continuous list is a labelled gap, and its rail
      // entry would lead nowhere — so it is omitted until something is picked.
      expect(find.byTooltip('Frequently used'), findsNothing);
      expect(find.text('Frequently used'), findsNothing);
      expect(find.byKey(const ValueKey('emoji-picker-grid')), findsOneWidget);
    });

    testWidgets('every section lives in one continuous scroll view', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs());

      // The old picker swapped the grid per tab, so only one category's emoji
      // existed at a time. Now they are all in the same list — the rail is a
      // shortcut into it, not a page switcher.
      final grid = find.byKey(const ValueKey('emoji-picker-grid'));
      expect(grid, findsOneWidget);
      expect(
        find.descendant(
          of: grid,
          matching: find.byKey(const ValueKey('emoji-tile-grinning')),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: grid,
          matching: find.byKey(const ValueKey('emoji-tile-fire')),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: grid,
          matching: find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
        ),
        findsOneWidget,
      );
    });

    testWidgets('the rail divides the full tray width evenly', (tester) async {
      await _pumpPicker(tester, prefs: await _prefs());

      // The rail used to be a short left-aligned strip. Every section now gets
      // one evenly-sized slot across the same width the search field spans.
      final searchField = tester.getRect(
        find.byKey(const ValueKey('emoji-picker-search')),
      );
      final people = tester.getRect(find.byTooltip('Smileys & People'));
      final nature = tester.getRect(find.byTooltip('Animals & Nature'));
      final custom = tester.getRect(find.byTooltip('Custom'));

      expect(nature.left, greaterThan(people.left));
      expect(custom.left, greaterThan(nature.left));
      expect(people.width, closeTo(nature.width, 0.5));
      expect(people.width, closeTo(custom.width, 0.5));
      // First slot starts and last slot ends on the search field's edges.
      expect(people.left, closeTo(searchField.left, 0.5));
      expect(custom.right, closeTo(searchField.right, 0.5));
    });

    testWidgets('tapping the rail scrolls the grid instead of replacing it', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs(), dataset: _tallDataset);

      final grid = find.byKey(const ValueKey('emoji-picker-grid'));
      double offset() =>
          tester.widget<CustomScrollView>(grid).controller!.offset;
      expect(offset(), 0);

      await tester.tap(find.byTooltip('Animals & Nature'));
      await tester.pumpAndSettle();

      // Same scroll view, moved — not a swapped-in second grid.
      expect(grid, findsOneWidget);
      expect(offset(), greaterThan(0));
      // People has 200 emoji at 8 per row: 25 rows of 40px plus a 28px header.
      expect(offset(), closeTo(28 + 25 * 40, 0.5));
    });

    testWidgets('the custom section only exists when the palette has emoji', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs(), customEmoji: const []);
      expect(find.byTooltip('Custom'), findsNothing);
      expect(
        find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
        findsNothing,
      );

      await _pumpPicker(tester, prefs: await _prefs());
      expect(find.byTooltip('Custom'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
        findsOneWidget,
      );
    });

    testWidgets('custom emoji sit in the same cell as native glyphs', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs());

      // A community's own emoji used to get their own looser 6-per-row grid,
      // which read as a different component bolted onto the sheet.
      final native = tester.getRect(
        find.byKey(const ValueKey('emoji-tile-fire')),
      );
      final custom = tester.getRect(
        find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
      );
      expect(custom.width, closeTo(native.width, 0.5));
      expect(custom.height, closeTo(native.height, 0.5));
    });

    testWidgets('typing filters across the standard and custom sets', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs());

      await tester.enterText(
        find.byKey(const ValueKey('emoji-picker-search')),
        'flame',
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('emoji-picker-search-results')),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('emoji-tile-fire')), findsOneWidget);
      expect(find.byKey(const ValueKey('emoji-tile-grinning')), findsNothing);
      // Searching hides the rail — the query is the navigation.
      expect(find.byTooltip('Smileys & People'), findsNothing);

      // Custom emoji rank in the same query, in their own section.
      await tester.enterText(
        find.byKey(const ValueKey('emoji-picker-search')),
        'parrot',
      );
      await tester.pumpAndSettle();
      expect(find.text('Custom'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
        findsOneWidget,
      );
    });

    testWidgets('crosses the shortcode separator emoji-mart cannot', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs());

      await tester.enterText(
        find.byKey(const ValueKey('emoji-picker-search')),
        'pointup',
      );
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('emoji-tile-point_up')), findsOneWidget);
    });

    testWidgets('reports no results rather than an empty grid', (tester) async {
      await _pumpPicker(tester, prefs: await _prefs());

      await tester.enterText(
        find.byKey(const ValueKey('emoji-picker-search')),
        'zzzzzz',
      );
      await tester.pumpAndSettle();

      expect(find.text('No emoji found.'), findsOneWidget);
    });

    testWidgets('clear button restores browsing', (tester) async {
      await _pumpPicker(tester, prefs: await _prefs());

      await tester.enterText(
        find.byKey(const ValueKey('emoji-picker-search')),
        'fire',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('emoji-picker-search-clear')));
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('emoji-picker-search-results')),
        findsNothing,
      );
      expect(find.byTooltip('Smileys & People'), findsOneWidget);
    });

    testWidgets('a standard emoji emits its glyph', (tester) async {
      final selected = await _pumpPicker(tester, prefs: await _prefs());

      await tester.tap(find.byKey(const ValueKey('emoji-tile-fire')));
      await tester.pumpAndSettle();

      expect(selected, ['\u{1F525}']);
    });

    testWidgets('a skin-tone variant is selectable', (tester) async {
      final selected = await _pumpPicker(tester, prefs: await _prefs());

      await tester.tap(find.byKey(const ValueKey('emoji-tile-point_up-1')));
      await tester.pumpAndSettle();

      expect(selected, ['\u{261D}\u{1F3FD}']);
    });

    testWidgets('a custom emoji emits :shortcode:', (tester) async {
      final selected = await _pumpPicker(tester, prefs: await _prefs());

      await tester.tap(
        find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
      );
      await tester.pumpAndSettle();

      expect(selected, [':partyparrot:']);
    });

    testWidgets('a non-reaction selection does not record a quick reaction', (
      tester,
    ) async {
      final prefs = await _prefs();
      final selected = await _pumpPicker(tester, prefs: prefs);

      await tester.tap(find.byKey(const ValueKey('emoji-tile-fire')));
      await tester.pumpAndSettle();

      expect(selected, ['\u{1F525}']);
      expect(
        prefs.getString(
          'buzz.quick-reaction-emojis.v1:http://localhost:3000:self',
        ),
        isNull,
      );
    });

    testWidgets('shows a spinner while the dataset is still loading', (
      tester,
    ) async {
      final prefs = await _prefs();
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            myPubkeyProvider.overrideWithValue('self'),
            emojiDatasetOrEmptyProvider.overrideWithValue(EmojiDataset.empty),
            customEmojiListProvider.overrideWithValue(const []),
          ],
          child: EmojiPickerSheet(onSelect: (_) {}),
        ),
      );
      // Not pumpAndSettle — the spinner animates forever.
      await tester.pump();

      expect(find.byType(CircularProgressIndicator), findsOneWidget);
    });
  });

  group('recent emoji ranking', () {
    test('promotes by use count, breaking ties on recency', () {
      var entries = <RecentEmojiEntry>[];
      entries = recordRecentEmoji(entries, 'a', now: 10);
      entries = recordRecentEmoji(entries, 'b', now: 20);
      entries = recordRecentEmoji(entries, 'b', now: 30);
      entries = recordRecentEmoji(entries, 'c', now: 40);

      expect(entries.map((entry) => entry.emoji), ['b', 'c', 'a']);
      expect(entries.first.count, 2);
    });

    test('tops the quick row up with the defaults', () {
      final entries = recordRecentEmoji(const [], '\u{1F525}', now: 1);
      final row = quickReactionEmoji(entries, customShortcodes: const {});

      expect(row.first, '\u{1F525}');
      expect(row, hasLength(5));
      expect(row, containsAll(defaultQuickEmojis.take(4)));
    });

    test('drops custom emoji no longer in the palette', () {
      final entries = recordRecentEmoji(const [], ':gone:', now: 1);

      expect(
        quickReactionEmoji(entries, customShortcodes: const {}),
        defaultQuickEmojis,
      );
      expect(
        quickReactionEmoji(entries, customShortcodes: const {'gone'}).first,
        ':gone:',
      );
    });
  });
}
