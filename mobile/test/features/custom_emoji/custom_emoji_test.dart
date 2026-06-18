import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/relay/nostr_models.dart';

NostrEvent _event(
  String pubkey,
  List<List<String>> emojiTags, {
  int createdAt = 0,
}) {
  return NostrEvent(
    id: 'id-$pubkey',
    pubkey: pubkey,
    createdAt: createdAt,
    kind: kindEmojiSet,
    tags: [
      ['d', customEmojiSetDTag],
      ...emojiTags,
    ],
    content: '',
    sig: '',
  );
}

void main() {
  group('normalizeShortcode', () {
    test('strips colons and lowercases', () {
      expect(normalizeShortcode(':PartyParrot:'), 'partyparrot');
      expect(normalizeShortcode('  meow  '), 'meow');
      expect(normalizeShortcode('a_b-c1'), 'a_b-c1');
    });

    test('rejects invalid chars and empty', () {
      expect(normalizeShortcode(':: ::'), isNull);
      expect(normalizeShortcode('has space'), isNull);
      expect(normalizeShortcode('emoji!'), isNull);
      expect(normalizeShortcode(''), isNull);
    });
  });

  group('customEmojiFromTags', () {
    test('parses valid emoji tags, normalizing shortcodes', () {
      final result = customEmojiFromTags([
        ['emoji', 'Meow', 'https://a/meow.png'],
        ['p', 'someone'],
        ['emoji', ':Wave:', 'https://a/wave.png'],
      ]);
      expect(result, [
        const CustomEmoji(shortcode: 'meow', url: 'https://a/meow.png'),
        const CustomEmoji(shortcode: 'wave', url: 'https://a/wave.png'),
      ]);
    });

    test('skips malformed and dup-within-event (first wins)', () {
      final result = customEmojiFromTags([
        ['emoji', 'meow'], // missing url
        ['emoji', '', 'https://a/x.png'], // empty shortcode
        ['emoji', 'meow', 'https://a/meow1.png'],
        ['emoji', 'meow', 'https://a/meow2.png'], // dup
      ]);
      expect(result, [
        const CustomEmoji(shortcode: 'meow', url: 'https://a/meow1.png'),
      ]);
    });
  });

  group('unionCustomEmoji', () {
    test(
      'collapses to one per shortcode; same-time tie-breaks to smaller URL',
      () {
        final palette = unionCustomEmoji([
          _event('alice', [
            ['emoji', 'meow', 'https://z/meow.png'],
            ['emoji', 'wave', 'https://a/wave.png'],
          ]),
          _event('bob', [
            ['emoji', 'meow', 'https://a/meow.png'], // tie → smaller URL wins
          ]),
        ]);
        expect(palette, [
          const CustomEmoji(shortcode: 'meow', url: 'https://a/meow.png'),
          const CustomEmoji(shortcode: 'wave', url: 'https://a/wave.png'),
        ]);
      },
    );

    test('most recently published set wins, regardless of URL order', () {
      final older = _event('alice', [
        ['emoji', 'x', 'https://a/x.png'],
      ], createdAt: 100);
      final newer = _event('bob', [
        ['emoji', 'x', 'https://z/x.png'],
      ], createdAt: 200);
      const expected = [CustomEmoji(shortcode: 'x', url: 'https://z/x.png')];
      expect(unionCustomEmoji([older, newer]), expected);
      expect(unionCustomEmoji([newer, older]), expected);
    });

    test('deterministic regardless of event order', () {
      final a = _event('alice', [
        ['emoji', 'x', 'https://z/x.png'],
      ]);
      final b = _event('bob', [
        ['emoji', 'x', 'https://a/x.png'],
      ]);
      expect(unionCustomEmoji([a, b]), unionCustomEmoji([b, a]));
    });

    test('empty input → empty palette', () {
      expect(unionCustomEmoji([]), isEmpty);
    });
  });

  group('buildCustomEmojiTags', () {
    final palette = [
      const CustomEmoji(shortcode: 'meow', url: 'https://a/meow.png'),
      const CustomEmoji(shortcode: 'wave', url: 'https://a/wave.png'),
    ];

    test(
      'emits one tag per distinct known shortcode, first-appearance order',
      () {
        final tags = buildCustomEmojiTags(
          'hi :wave: and :meow: and :wave:',
          palette,
        );
        expect(tags, [
          ['emoji', 'wave', 'https://a/wave.png'],
          ['emoji', 'meow', 'https://a/meow.png'],
        ]);
      },
    );

    test('case-insensitive match, canonical lowercase emitted', () {
      final tags = buildCustomEmojiTags(':MEOW:', palette);
      expect(tags, [
        ['emoji', 'meow', 'https://a/meow.png'],
      ]);
    });

    test('ignores unknown shortcodes', () {
      expect(buildCustomEmojiTags(':unknown:', palette), isEmpty);
    });

    test('empty palette → no tags', () {
      expect(buildCustomEmojiTags(':meow:', const []), isEmpty);
    });
  });

  group('reactionEmojiUrl', () {
    final palette = [
      const CustomEmoji(shortcode: 'meow', url: 'https://a/meow.png'),
    ];

    test('resolves a known custom-emoji reaction', () {
      expect(reactionEmojiUrl(':meow:', palette), 'https://a/meow.png');
      expect(reactionEmojiUrl(':MEOW:', palette), 'https://a/meow.png');
    });

    test('returns null for unicode / unknown / no palette', () {
      expect(reactionEmojiUrl('👍', palette), isNull);
      expect(reactionEmojiUrl(':unknown:', palette), isNull);
      expect(reactionEmojiUrl(':meow:', null), isNull);
    });
  });
}
