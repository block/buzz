import 'dart:async';

import 'package:buzz/features/channels/emoji_picker.dart';
import 'package:buzz/features/channels/recent_emoji_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:nostr/nostr.dart' as nostr;

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
      native: '\u{261D}\u{1F3FB}',
      categoryId: 'people',
      name: 'Index Pointing Up',
      skinIndex: 1,
    ),
    _entry(
      'point_up',
      native: '\u{261D}\u{1F3FC}',
      categoryId: 'people',
      name: 'Index Pointing Up',
      skinIndex: 2,
    ),
    _entry(
      'point_up',
      native: '\u{261D}\u{1F3FD}',
      categoryId: 'people',
      name: 'Index Pointing Up',
      skinIndex: 3,
    ),
    _entry(
      'point_up',
      native: '\u{261D}\u{1F3FE}',
      categoryId: 'people',
      name: 'Index Pointing Up',
      skinIndex: 4,
    ),
    _entry(
      'point_up',
      native: '\u{261D}\u{1F3FF}',
      categoryId: 'people',
      name: 'Index Pointing Up',
      skinIndex: 5,
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
const _relayCustomEmoji = [
  CustomEmoji(
    shortcode: 'buzzbee',
    url: 'https://relay.example/media/buzzbee.png',
  ),
];

class _FakeCustomEmojiPaletteNotifier extends CustomEmojiPaletteNotifier {
  _FakeCustomEmojiPaletteNotifier(this.palette);

  final Future<List<CustomEmoji>> palette;

  @override
  Future<List<CustomEmoji>> build() => palette;
}

const _nativeEmojiPickerChannel = MethodChannel('buzz/native_emoji_picker');

void _setMockNativeEmojiPickerHandler(
  Future<Object?> Function(MethodCall call)? handler,
) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(_nativeEmojiPickerChannel, handler);
}

Future<Object?> _sendNativeEmojiPickerCall(
  WidgetTester tester,
  String method, [
  Object? arguments,
]) async {
  final response = Completer<ByteData?>();
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    _nativeEmojiPickerChannel.name,
    _nativeEmojiPickerChannel.codec.encodeMethodCall(
      MethodCall(method, arguments),
    ),
    response.complete,
  );
  final envelope = await response.future;
  return envelope == null
      ? null
      : _nativeEmojiPickerChannel.codec.decodeEnvelope(envelope);
}

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
      final sectionKeys = tester
          .widget<CustomScrollView>(grid)
          .slivers
          .whereType<SliverMainAxisGroup>()
          .map((sliver) => sliver.key);
      expect(
        sectionKeys,
        containsAllInOrder(const [
          ValueKey('emoji-section-people'),
          ValueKey('emoji-section-nature'),
          ValueKey('emoji-section-custom'),
        ]),
      );
    });

    testWidgets(
      'search shares the sheet header with the shared close control',
      (tester) async {
        await _pumpPicker(tester, prefs: await _prefs());

        final search = tester.getRect(
          find.byKey(const ValueKey('emoji-picker-search')),
        );
        final close = tester.getRect(find.byTooltip('Close sheet'));

        expect(close.size, const Size.square(44));
        expect(search.center.dy, close.center.dy);
        expect(close.left - search.right, Grid.xxs);
      },
    );

    testWidgets('the Flutter picker keeps the established tray height', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs());

      final picker = find.byType(EmojiPickerSheet);
      final context = tester.element(picker);
      expect(
        tester.getSize(picker).height,
        closeTo(MediaQuery.sizeOf(context).height * 0.62, 0.5),
      );
      expect(find.byType(DraggableScrollableSheet), findsNothing);
    });

    testWidgets('the Flutter search field is a full pill', (tester) async {
      await _pumpPicker(tester, prefs: await _prefs());

      final field = tester.widget<TextField>(
        find.byKey(const ValueKey('emoji-picker-search')),
      );
      final border = field.decoration!.border! as OutlineInputBorder;
      expect(border.borderRadius, BorderRadius.circular(Radii.full));
    });

    testWidgets('uses the shared sheet surface instead of a picker override', (
      tester,
    ) async {
      final prefs = await _prefs();
      final theme = AppTheme.light().copyWith(
        colorScheme: lightColorScheme.copyWith(
          surfaceContainerHighest: Colors.grey,
        ),
        bottomSheetTheme: const BottomSheetThemeData(
          backgroundColor: Colors.green,
        ),
      );
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            myPubkeyProvider.overrideWithValue('self'),
            emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
            customEmojiListProvider.overrideWithValue(_customEmoji),
          ],
          child: MaterialApp(
            theme: theme,
            home: Scaffold(
              body: Builder(
                builder: (context) => FilledButton(
                  onPressed: () =>
                      showEmojiPicker(context: context, onSelect: (_) {}),
                  child: const Text('Open picker'),
                ),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Open picker'));
      await tester.pumpAndSettle();

      final ancestorMaterials = find
          .ancestor(
            of: find.byType(EmojiPickerSheet),
            matching: find.byType(Material),
          )
          .evaluate()
          .map((element) => element.widget as Material);
      expect(
        ancestorMaterials.map((material) => material.color),
        contains(Colors.green),
      );
      expect(
        ancestorMaterials.map((material) => material.color),
        isNot(contains(Colors.grey)),
      );
    });

    testWidgets('the rail divides the full tray width evenly', (tester) async {
      await _pumpPicker(tester, prefs: await _prefs());

      // The rail used to be a short left-aligned strip. Every section now gets
      // one evenly-sized slot across the tray, while search shares its row with
      // the close control above.
      final picker = tester.getRect(find.byType(EmojiPickerSheet));
      final people = tester.getRect(find.byTooltip('Smileys & People'));
      final nature = tester.getRect(find.byTooltip('Animals & Nature'));
      final custom = tester.getRect(find.byTooltip('Custom'));
      final skinTone = tester.getRect(find.byTooltip('Skin tone'));

      expect(nature.left, greaterThan(people.left));
      expect(custom.left, greaterThan(nature.left));
      expect(people.width, closeTo(nature.width, 0.5));
      expect(people.width, closeTo(custom.width, 0.5));
      expect(people.width, closeTo(skinTone.width, 0.5));
      expect(people.left, closeTo(picker.left + Grid.gutter, 0.5));
      expect(skinTone.right, closeTo(picker.right - Grid.gutter, 0.5));
      expect(
        tester.getSize(
          find.byKey(const ValueKey('emoji-skin-tone-dot-selected')),
        ),
        const Size.square(16),
      );
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
      await tester.pump();

      // Same scroll view, moved — not a swapped-in second grid.
      expect(grid, findsOneWidget);
      expect(offset(), greaterThan(0));
      // People has 200 emoji at 8 per row: 25 rows of 40px plus a 28px header.
      expect(offset(), closeTo(28 + 25 * 40, 0.5));
      expect(
        tester
            .widget<CustomScrollView>(grid)
            .controller!
            .position
            .isScrollingNotifier
            .value,
        isFalse,
      );
    });

    testWidgets(
      'a bottom-clamped final section stays highlighted after a rail tap',
      (tester) async {
        await _pumpPicker(tester, prefs: await _prefs(), dataset: _tallDataset);
        final colors = Theme.of(
          tester.element(find.byType(EmojiPickerSheet)),
        ).colorScheme;
        Color iconColor(String tooltip) => tester
            .widget<Icon>(
              find.descendant(
                of: find.byTooltip(tooltip),
                matching: find.byType(Icon),
              ),
            )
            .color!;

        await tester.tap(find.byTooltip('Custom'));
        await tester.pumpAndSettle();

        final grid = tester.widget<CustomScrollView>(
          find.byKey(const ValueKey('emoji-picker-grid')),
        );
        expect(
          grid.controller!.offset,
          closeTo(grid.controller!.position.maxScrollExtent, 0.5),
        );
        expect(iconColor('Custom'), colors.primary);
        expect(iconColor('Animals & Nature'), colors.onSurfaceVariant);
      },
    );

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
      await tester.tap(find.byTooltip('Custom'));
      await tester.pumpAndSettle();
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
      final nativeWidth = tester
          .getRect(find.byKey(const ValueKey('emoji-tile-grinning')))
          .width;
      await tester.tap(find.byTooltip('Custom'));
      await tester.pumpAndSettle();
      final custom = tester.getRect(
        find.byKey(const ValueKey('emoji-tile-custom-partyparrot')),
      );
      expect(custom.width, closeTo(nativeWidth, 0.5));
      expect(custom.height, closeTo(40, 0.5));
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

    testWidgets(
      'changing skin tone keeps the scrolled-to category highlighted',
      (tester) async {
        // A skin-tone change rebuilds the sections and the active-section
        // notifier. Regression: the notifier was recreated at index 0, so a
        // user parked on a later category snapped back to the first one in the
        // rail while the grid stayed put. The notifier now seeds from the live
        // scroll offset, so the highlight survives the rebuild.
        await _pumpPicker(tester, prefs: await _prefs(), dataset: _tallDataset);
        final colors = Theme.of(
          tester.element(find.byType(EmojiPickerSheet)),
        ).colorScheme;
        Color iconColor(String tooltip) => tester
            .widget<Icon>(
              find.descendant(
                of: find.byTooltip(tooltip),
                matching: find.byType(Icon),
              ),
            )
            .color!;

        await tester.tap(find.byTooltip('Animals & Nature'));
        await tester.pumpAndSettle();
        expect(iconColor('Animals & Nature'), colors.primary);
        expect(iconColor('Smileys & People'), colors.onSurfaceVariant);

        await tester.tap(find.byTooltip('Skin tone'));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('emoji-skin-tone-3')));
        await tester.pumpAndSettle();

        // Still on Nature after the tone rebuild — not reset to People.
        expect(iconColor('Animals & Nature'), colors.primary);
        expect(iconColor('Smileys & People'), colors.onSurfaceVariant);
      },
    );

    testWidgets('a standard emoji emits its glyph', (tester) async {
      final selected = await _pumpPicker(tester, prefs: await _prefs());

      await tester.tap(find.byTooltip('Animals & Nature'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('emoji-tile-fire')));
      await tester.pumpAndSettle();

      expect(selected, ['\u{1F525}']);
    });

    testWidgets('skin tone choice shows and emits one selected variant', (
      tester,
    ) async {
      final selected = await _pumpPicker(tester, prefs: await _prefs());

      expect(find.byKey(const ValueKey('emoji-tile-point_up')), findsOneWidget);
      expect(find.byKey(const ValueKey('emoji-tile-point_up-1')), findsNothing);

      await tester.tap(find.byTooltip('Skin tone'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('emoji-skin-tone-3')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('emoji-tile-point_up')));
      await tester.pumpAndSettle();

      expect(selected, ['\u{261D}\u{1F3FD}']);
    });

    testWidgets('skin tone choices paint their colors on a solid layer', (
      tester,
    ) async {
      await _pumpPicker(tester, prefs: await _prefs());

      await tester.tap(find.byTooltip('Skin tone'));
      await tester.pumpAndSettle();

      const expectedColors = [
        Color(0xFFFFC93A),
        Color(0xFFFFDAB7),
        Color(0xFFE7B98F),
        Color(0xFFC88C61),
        Color(0xFFA46134),
        Color(0xFF5D4437),
      ];
      for (final (index, expectedColor) in expectedColors.indexed) {
        final decorations = tester
            .widgetList<DecoratedBox>(
              find.descendant(
                of: find.byKey(ValueKey('emoji-skin-tone-dot-$index')),
                matching: find.byType(DecoratedBox),
              ),
            )
            .map((widget) => widget.decoration)
            .whereType<BoxDecoration>();

        expect(
          decorations.any(
            (decoration) =>
                decoration.color == expectedColor &&
                decoration.gradient == null &&
                decoration.backgroundBlendMode == null,
          ),
          isTrue,
        );
      }
    });

    testWidgets('a custom emoji emits :shortcode:', (tester) async {
      final selected = await _pumpPicker(tester, prefs: await _prefs());

      await tester.tap(find.byTooltip('Custom'));
      await tester.pumpAndSettle();
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

      await tester.tap(find.byTooltip('Animals & Nature'));
      await tester.pumpAndSettle();
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

  group('iOS native emoji picker', () {
    setUp(resetIosEmojiPickerPresentationForTest);

    testWidgets('passes shared theme and custom emoji into the native sheet', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final prefs = await _prefs();
      final mediaAuth = MediaGetAuthService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
      );
      MethodCall? presentation;
      final selected = <String>[];
      var dismissals = 0;
      _setMockNativeEmojiPickerHandler((call) async {
        presentation = call;
        return true;
      });

      try {
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              myPubkeyProvider.overrideWithValue('self'),
              customEmojiPaletteProvider.overrideWith(
                () => _FakeCustomEmojiPaletteNotifier(
                  Future.value([..._customEmoji, ..._relayCustomEmoji]),
                ),
              ),
              mediaGetAuthServiceProvider.overrideWithValue(mediaAuth),
            ],
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(
                body: Builder(
                  builder: (context) => FilledButton(
                    onPressed: () => showEmojiPicker(
                      context: context,
                      onSelect: selected.add,
                      onDismiss: () => dismissals += 1,
                    ),
                    child: const Text('Open picker'),
                  ),
                ),
              ),
            ),
          ),
        );

        await tester.tap(find.text('Open picker'));
        await tester.pump();

        expect(presentation?.method, 'present');
        final arguments = presentation?.arguments as Map<Object?, Object?>;
        expect(arguments['surfaceColor'], lightColorScheme.surface.toARGB32());
        expect(arguments['skinTone'], 0);
        expect(arguments['customEmoji'], [
          {
            'shortcode': 'partyparrot',
            'url': 'https://example.test/parrot.gif',
          },
          {
            'shortcode': 'buzzbee',
            'url': 'https://relay.example/media/buzzbee.png',
          },
        ]);
        expect(find.byType(EmojiPickerSheet), findsNothing);

        final externalHeaders = await _sendNativeEmojiPickerCall(
          tester,
          'mediaHeaders',
          'https://example.test/parrot.gif',
        );
        expect(externalHeaders, <String, String>{});
        final relayHeaders = await _sendNativeEmojiPickerCall(
          tester,
          'mediaHeaders',
          'https://relay.example/media/buzzbee.png',
        );
        expect(
          (relayHeaders as Map<Object?, Object?>)['Authorization'],
          startsWith('Nostr '),
        );

        await _sendNativeEmojiPickerCall(tester, 'skinToneChanged', 4);
        await _sendNativeEmojiPickerCall(tester, 'selected', '\u{1F525}');
        await _sendNativeEmojiPickerCall(tester, 'dismissed');
        expect(selected, ['\u{1F525}']);
        expect(dismissals, 1);
        expect(prefs.getInt('buzz.emoji-picker.skin-tone.v1'), 4);
      } finally {
        _setMockNativeEmojiPickerHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('shows cancellable feedback while loading the custom palette', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final prefs = await _prefs();
      final palette = Completer<List<CustomEmoji>>();
      MethodCall? presentation;
      _setMockNativeEmojiPickerHandler((call) async {
        presentation = call;
        return true;
      });

      try {
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              customEmojiPaletteProvider.overrideWith(
                () => _FakeCustomEmojiPaletteNotifier(palette.future),
              ),
            ],
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(
                body: Builder(
                  builder: (context) => FilledButton(
                    onPressed: () =>
                        showEmojiPicker(context: context, onSelect: (_) {}),
                    child: const Text('Open picker'),
                  ),
                ),
              ),
            ),
          ),
        );

        await tester.tap(find.text('Open picker'));
        await tester.pump();
        expect(presentation, isNull);
        expect(
          find.byKey(const Key('ios-emoji-picker-palette-loading')),
          findsOneWidget,
        );
        expect(find.text('Loading emoji…'), findsOneWidget);

        palette.complete(_customEmoji);
        await tester.pumpAndSettle();
        expect(
          find.byKey(const Key('ios-emoji-picker-palette-loading')),
          findsNothing,
        );
        expect(presentation?.method, 'present');
      } finally {
        _setMockNativeEmojiPickerHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('cancels palette loading without presenting native picker', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final prefs = await _prefs();
      final palette = Completer<List<CustomEmoji>>();
      var presents = 0;
      var dismissals = 0;
      _setMockNativeEmojiPickerHandler((call) async {
        if (call.method == 'present') presents += 1;
        return true;
      });

      try {
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              customEmojiPaletteProvider.overrideWith(
                () => _FakeCustomEmojiPaletteNotifier(palette.future),
              ),
            ],
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(
                body: Builder(
                  builder: (context) => FilledButton(
                    onPressed: () => showEmojiPicker(
                      context: context,
                      onSelect: (_) {},
                      onDismiss: () => dismissals += 1,
                    ),
                    child: const Text('Open picker'),
                  ),
                ),
              ),
            ),
          ),
        );

        await tester.tap(find.text('Open picker'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 300));
        expect(
          find.byKey(const Key('ios-emoji-picker-palette-loading')),
          findsOneWidget,
        );

        await tester.tapAt(const Offset(20, 20));
        await tester.pumpAndSettle();
        expect(dismissals, 1);
        expect(presents, 0);

        palette.complete(_customEmoji);
        await tester.pumpAndSettle();
        expect(presents, 0);
        expect(dismissals, 1);
      } finally {
        _setMockNativeEmojiPickerHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets('falls back to the Flutter picker when native cannot present', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final prefs = await _prefs();
      _setMockNativeEmojiPickerHandler((_) async => false);

      try {
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              myPubkeyProvider.overrideWithValue('self'),
              emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
              customEmojiPaletteProvider.overrideWith(
                () =>
                    _FakeCustomEmojiPaletteNotifier(Future.value(_customEmoji)),
              ),
            ],
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(
                body: Builder(
                  builder: (context) => FilledButton(
                    onPressed: () =>
                        showEmojiPicker(context: context, onSelect: (_) {}),
                    child: const Text('Open picker'),
                  ),
                ),
              ),
            ),
          ),
        );

        await tester.tap(find.text('Open picker'));
        await tester.pumpAndSettle();
        expect(find.byType(EmojiPickerSheet), findsOneWidget);
      } finally {
        await tester.pumpWidget(const SizedBox.shrink());
        _setMockNativeEmojiPickerHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets(
      'a palette load failure falls back to the Flutter picker exactly once',
      (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        final prefs = await _prefs();
        var presents = 0;
        _setMockNativeEmojiPickerHandler((_) async {
          presents += 1;
          return true;
        });
        var dismissals = 0;

        try {
          await tester.pumpWidget(
            ProviderScope(
              overrides: [
                savedPrefsProvider.overrideWithValue(prefs),
                myPubkeyProvider.overrideWithValue('self'),
                emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
                customEmojiPaletteProvider.overrideWith(
                  () => _FakeCustomEmojiPaletteNotifier(
                    Future.error(StateError('palette unavailable')),
                  ),
                ),
              ],
              child: MaterialApp(
                theme: AppTheme.light(),
                home: Scaffold(
                  body: Builder(
                    builder: (context) => FilledButton(
                      onPressed: () => showEmojiPicker(
                        context: context,
                        onSelect: (_) {},
                        onDismiss: () => dismissals += 1,
                      ),
                      child: const Text('Open picker'),
                    ),
                  ),
                ),
              ),
            ),
          );

          await tester.tap(find.text('Open picker'));
          await tester.pumpAndSettle();

          // The native sheet is never presented on a palette error; the
          // Flutter picker takes over so the composer's open state is not
          // stranded.
          expect(presents, 0);
          expect(find.byType(EmojiPickerSheet), findsOneWidget);

          // Dismissing the fallback runs onDismiss exactly once.
          await tester.tapAt(const Offset(20, 20));
          await tester.pumpAndSettle();
          expect(dismissals, 1);
        } finally {
          await tester.pumpWidget(const SizedBox.shrink());
          _setMockNativeEmojiPickerHandler(null);
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      },
    );

    testWidgets('a reentrant open cannot steal the live sheet callbacks', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final prefs = await _prefs();
      var presents = 0;
      _setMockNativeEmojiPickerHandler((call) async {
        if (call.method == 'present') presents += 1;
        return true;
      });

      final firstSelected = <String>[];
      final secondSelected = <String>[];
      var firstDismissals = 0;
      var secondDismissals = 0;

      try {
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              myPubkeyProvider.overrideWithValue('self'),
              customEmojiPaletteProvider.overrideWith(
                () =>
                    _FakeCustomEmojiPaletteNotifier(Future.value(_customEmoji)),
              ),
            ],
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(
                body: Builder(
                  builder: (context) => Column(
                    children: [
                      FilledButton(
                        onPressed: () => showEmojiPicker(
                          context: context,
                          onSelect: firstSelected.add,
                          onDismiss: () => firstDismissals += 1,
                        ),
                        child: const Text('Open first'),
                      ),
                      FilledButton(
                        onPressed: () => showEmojiPicker(
                          context: context,
                          onSelect: secondSelected.add,
                          onDismiss: () => secondDismissals += 1,
                        ),
                        child: const Text('Open second'),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );

        await tester.tap(find.text('Open first'));
        await tester.pumpAndSettle();
        expect(presents, 1);

        // A second open while the first sheet is live is rejected: it neither
        // presents again nor replaces the live sheet's method-call handler,
        // and its independent lifecycle is completed immediately.
        await tester.tap(find.text('Open second'));
        await tester.pumpAndSettle();
        expect(presents, 1);
        expect(secondDismissals, 1);

        // Native events still reach the original owner, and only it.
        await _sendNativeEmojiPickerCall(tester, 'selected', '\u{1F525}');
        await _sendNativeEmojiPickerCall(tester, 'dismissed');
        expect(firstSelected, ['\u{1F525}']);
        expect(secondSelected, isEmpty);
        expect(firstDismissals, 1);
        expect(secondDismissals, 1);
      } finally {
        _setMockNativeEmojiPickerHandler(null);
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
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
