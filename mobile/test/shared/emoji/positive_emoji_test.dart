import 'package:buzz/shared/emoji/positive_emoji.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('isPositiveEmojiParticle', () {
    test('accepts each group desktop celebrates', () {
      expect(isPositiveEmojiParticle('\u{1F44D}'), isTrue); // 👍
      expect(isPositiveEmojiParticle('\u{1F389}'), isTrue); // 🎉
      expect(isPositiveEmojiParticle('\u{2764}\u{FE0F}'), isTrue); // ❤️
      expect(isPositiveEmojiParticle('\u{1F602}'), isTrue); // 😂
    });

    test('rejects everything outside the set', () {
      expect(isPositiveEmojiParticle('\u{1F44E}'), isFalse); // 👎
      expect(isPositiveEmojiParticle('\u{1F622}'), isFalse); // 😢
      expect(isPositiveEmojiParticle('\u{1F440}'), isFalse); // 👀
      expect(isPositiveEmojiParticle(':partyparrot:'), isFalse);
      expect(isPositiveEmojiParticle(''), isFalse);
    });

    test('ignores the presentation selector and surrounding space', () {
      // The same emoji arrives with and without U+FE0F depending on the
      // producer, and reactions come off the wire as well as from our picker.
      expect(isPositiveEmojiParticle('\u{2764}'), isTrue);
      expect(isPositiveEmojiParticle('\u{2B50}\u{FE0F}'), isTrue);
      expect(isPositiveEmojiParticle(' \u{1F525} '), isTrue);
    });

    test('every entry in the exported list passes its own gate', () {
      for (final emoji in positiveEmojiParticles) {
        expect(
          isPositiveEmojiParticle(emoji),
          isTrue,
          reason: 'expected $emoji to be a positive particle',
        );
      }
    });

    test('the three groups match desktop counts', () {
      // Guards against a half-applied port: desktop has 23 hearts, 14 reaction
      // emoji, and 15 faces.
      expect(heartParticleEmojis, hasLength(23));
      expect(positiveReactionParticleEmojis, hasLength(14));
      expect(positiveFaceParticleEmojis, hasLength(15));
      expect(positiveEmojiParticles, hasLength(52));
    });
  });
}
