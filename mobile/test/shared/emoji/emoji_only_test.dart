import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/emoji/emoji_only.dart';
import 'package:flutter_test/flutter_test.dart';

const _customEmoji = [
  CustomEmoji(shortcode: 'buzz', url: 'https://example.test/buzz.png'),
];

/// Keycaps carry no `Extended_Pictographic` code point of their own, so they
/// only pass via the dataset's glyph set — the one case that needs it.
const _keycapOne = '1\u{FE0F}\u{20E3}';

bool emojiOnly(
  String content, {
  Set<String> nativeEmoji = const {},
  List<CustomEmoji> customEmoji = const [],
}) => isEmojiOnlyMessage(
  content,
  nativeEmoji: nativeEmoji,
  customEmoji: customEmoji,
);

void main() {
  group('isEmojiOnlyMessage', () {
    test('accepts emoji, alone and in runs', () {
      // Mirrors the cases in desktop's emojiOnly.test.mjs.
      expect(emojiOnly('\u{1F600}'), isTrue);
      expect(
        emojiOnly('\u{1F600} \u{1F44D}\u{1F3FD}\n\u{2764}\u{FE0F}'),
        isTrue,
      );
      expect(emojiOnly('\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}'), isTrue);
      expect(
        emojiOnly(
          '\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}',
        ),
        isTrue,
      );
    });

    test('rejects anything with text in it', () {
      expect(emojiOnly('hi \u{1F600}'), isFalse);
      expect(emojiOnly('\u{1F600}!'), isFalse);
      expect(emojiOnly('nice'), isFalse);
    });

    test('rejects empty and whitespace-only bodies', () {
      expect(emojiOnly(''), isFalse);
      expect(emojiOnly('   \n '), isFalse);
    });

    test('counts a known custom shortcode as emoji', () {
      expect(emojiOnly(':buzz:', customEmoji: _customEmoji), isTrue);
      expect(emojiOnly(':buzz: \u{1F600}', customEmoji: _customEmoji), isTrue);
      expect(emojiOnly('hi :buzz:', customEmoji: _customEmoji), isFalse);
    });

    test('an unknown shortcode is just text', () {
      // Desktop behaves the same: without the palette it renders literally, so
      // it must not be scaled up as if it were a glyph.
      expect(emojiOnly(':buzz:'), isFalse);
      expect(emojiOnly(':nope:', customEmoji: _customEmoji), isFalse);
      expect(emojiOnly(':', customEmoji: _customEmoji), isFalse);
      expect(emojiOnly('::', customEmoji: _customEmoji), isFalse);
    });

    test('keycaps need the dataset glyph set', () {
      expect(emojiOnly(_keycapOne), isFalse);
      expect(emojiOnly(_keycapOne, nativeEmoji: {_keycapOne}), isTrue);
    });
  });

  group('emoji-only sizing', () {
    test('matches desktop’s step up from body text', () {
      // Desktop goes text-sm (14px) to text-4xl; mobile body is the same 14px.
      expect(kEmojiOnlyFontSize, 36.0);
      // Desktop sizes inline custom emoji at 1.45em of the surrounding text.
      expect(kEmojiOnlyCustomEmojiSize, closeTo(36.0 * 1.45, 0.001));
    });
  });
}
