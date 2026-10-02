import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_search.dart';
import 'package:flutter_test/flutter_test.dart';

EmojiEntry _entry(
  String id, {
  String? name,
  List<String> keywords = const [],
  String native = '*',
}) {
  return EmojiEntry(
    id: id,
    name: name ?? id,
    keywords: keywords,
    native: native,
    categoryId: 'people',
  );
}

void main() {
  group('collapseSeparators', () {
    test('lowercases and drops shortcode punctuation', () {
      expect(collapseSeparators(':Point_Up:'), 'pointup');
      expect(collapseSeparators('white-check-mark'), 'whitecheckmark');
      expect(collapseSeparators('  thumbs up '), 'thumbsup');
    });
  });

  group('scoreShortcodeMatch', () {
    test('ranks exact above prefix above substring above subsequence', () {
      final exact = scoreShortcodeMatch('point_up', 'point_up')!;
      final prefix = scoreShortcodeMatch('point', 'point_up')!;
      final substring = scoreShortcodeMatch('up', 'point_up')!;
      final subsequence = scoreShortcodeMatch('pntup', 'point_up')!;

      expect(exact.tier, lessThan(prefix.tier));
      expect(prefix.tier, lessThan(substring.tier));
      expect(substring.tier, lessThan(subsequence.tier));
    });

    test('crosses the separator emoji-mart cannot', () {
      expect(scoreShortcodeMatch('pointup', 'point_up'), isNotNull);
      expect(scoreShortcodeMatch('pointup', 'point_up')!.tier, 0);
    });

    test('returns null for an empty query or no match', () {
      expect(scoreShortcodeMatch('', 'point_up'), isNull);
      expect(scoreShortcodeMatch('   ', 'point_up'), isNull);
      expect(scoreShortcodeMatch('zzzz', 'point_up'), isNull);
    });
  });

  group('searchEmoji', () {
    final entries = [
      _entry(
        'point_up',
        name: 'Index Pointing Up',
        keywords: ['point', 'hand'],
      ),
      _entry('fire', name: 'Fire', keywords: ['flame', 'hot', 'lit']),
      _entry(
        'fire_engine',
        name: 'Fire Engine',
        keywords: ['fire', 'truck', 'emergency'],
      ),
      _entry('firecracker', name: 'Firecracker', keywords: ['dynamite']),
      _entry('smile', name: 'Grinning Face With Smiling Eyes'),
    ];

    test('exact shortcode wins over longer prefixes', () {
      final results = searchEmoji('fire', entries);
      expect(results.first.id, 'fire');
      expect(
        results.map((entry) => entry.id),
        containsAll(['fire', 'fire_engine', 'firecracker']),
      );
    });

    test('shortcode prefix outranks a keyword-only hit', () {
      final results = searchEmoji('flame', entries);
      expect(results.single.id, 'fire');
    });

    test('separator-insensitive shortcode search still works', () {
      expect(searchEmoji('pointup', entries).first.id, 'point_up');
      expect(searchEmoji('fireengine', entries).first.id, 'fire_engine');
    });

    test('matches on name words when the shortcode does not contain them', () {
      expect(searchEmoji('grinning', entries).single.id, 'smile');
    });

    test('honors the limit and returns empty for a blank query', () {
      expect(searchEmoji('fire', entries, limit: 2), hasLength(2));
      expect(searchEmoji('fire', entries, limit: 0), isEmpty);
      expect(searchEmoji('', entries), isEmpty);
    });
  });

  group('rankByShortcode', () {
    test('ranks custom-emoji shortcodes with the desktop tiers', () {
      final ranked = rankByShortcode('parrot', const [
        'party_parrot',
        'parrot',
        'pirate_parrot',
      ], (code) => code);

      expect(ranked.first, 'parrot');
      expect(ranked, hasLength(3));
    });

    test('prefers the shorter shortcode within a tier', () {
      final ranked = rankByShortcode('par', const [
        'parrotparrot',
        'park',
      ], (code) => code);

      expect(ranked.first, 'park');
    });
  });
}
