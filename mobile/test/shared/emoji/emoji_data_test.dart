import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:flutter_test/flutter_test.dart';

/// Reads the committed asset straight off disk rather than through
/// `rootBundle`, so this test guards the generated file itself — if
/// `just mobile-emoji-data` ever emits a different shape, this fails.
EmojiDataset _loadFromDisk() {
  final source = File(emojiDataAssetPath).readAsStringSync();
  return EmojiDataset.fromJson(jsonDecode(source) as Map<String, dynamic>);
}

void main() {
  late EmojiDataset dataset;

  setUpAll(() {
    dataset = _loadFromDisk();
  });

  test('parses every emoji-mart category in order', () {
    expect(dataset.categories.map((category) => category.id), [
      'people',
      'nature',
      'foods',
      'activity',
      'places',
      'objects',
      'symbols',
      'flags',
    ]);
  });

  test('carries the full standard set, not a hand-picked subset', () {
    // The hardcoded list this replaced had ~140 glyphs; emoji-mart ships ~1.9k.
    expect(dataset.all.length, greaterThan(1800));
    expect(
      dataset.all.length,
      dataset.categories.fold<int>(
        0,
        (total, category) => total + category.emoji.length,
      ),
    );
  });

  test('every entry has an id, a glyph, and a category', () {
    for (final entry in dataset.all) {
      expect(entry.id, isNotEmpty);
      expect(entry.native, isNotEmpty);
      expect(entry.categoryId, isNotEmpty);
    }
  });

  test('resolves glyphs back to desktop-identical shortcodes', () {
    expect(dataset.nativeToShortcode['💯'], ':100:');
    expect(dataset.nativeToShortcode['🔥'], ':fire:');
    expect(dataset.nativeToShortcode['☝️'], ':point_up:');
    expect(dataset.nativeToShortcode['👍🏽'], ':+1:');
    expect(
      dataset.all
          .where((entry) => entry.id == '+1')
          .map((entry) => entry.native),
      containsAll(['👍', '👍🏻', '👍🏼', '👍🏽', '👍🏾', '👍🏿']),
    );
  });

  test('displayName mirrors desktop emojiDisplayName', () {
    expect(dataset.displayName('🎉'), ':tada:');
    // Custom emoji already arrive as a shortcode and pass through untouched.
    expect(dataset.displayName(':party_parrot:'), ':party_parrot:');
    // Unknown glyphs fall back to themselves rather than a bogus name.
    expect(dataset.displayName('\u{1FAF6}\u{200D}\u{2764}'), isNotEmpty);
  });

  test('empty dataset degrades safely', () {
    expect(EmojiDataset.empty.isEmpty, isTrue);
    expect(EmojiDataset.empty.displayName('🔥'), '🔥');
  });
}
