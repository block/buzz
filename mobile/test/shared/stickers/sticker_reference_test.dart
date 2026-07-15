import 'package:buzz/shared/stickers/sticker_reference.dart';
import 'package:flutter_test/flutter_test.dart';

const _authorUpper =
    'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA';
const _authorLower =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _hashUpper =
    'BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB';
const _hashLower =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

void main() {
  group('parseStickerReference', () {
    test('returns absent when the message has no sticker tag', () {
      final result = parseStickerReference(const [
        ['h', 'channel-id'],
      ]);

      expect(result.status, StickerTagStatus.absent);
      expect(result.reference, isNull);
    });

    test('parses one canonical lowercase tag', () {
      final result = parseStickerReference(const [
        ['h', 'channel-id'],
        ['sticker', '30031:$_authorLower:cats.fun', 'Wave_1', _hashLower],
      ]);

      expect(result.status, StickerTagStatus.valid);
      expect(result.reference?.authorPubkey, _authorLower);
      expect(result.reference?.packIdentifier, 'cats.fun');
      expect(result.reference?.shortcode, 'Wave_1');
      expect(result.reference?.sha256, _hashLower);
      expect(result.reference?.coordinate, '30031:$_authorLower:cats.fun');
    });

    test('rejects non-canonical uppercase hex', () {
      final result = parseStickerReference(const [
        ['sticker', '30031:$_authorUpper:cats.fun', 'Wave_1', _hashUpper],
      ]);

      expect(result.status, StickerTagStatus.invalid);
      expect(result.reference, isNull);
    });

    test('rejects more than one sticker tag', () {
      final result = parseStickerReference(const [
        ['sticker', '30031:$_authorLower:cats', 'wave', _hashLower],
        ['sticker', '30031:$_authorLower:cats', 'smile', _hashLower],
      ]);

      expect(result.status, StickerTagStatus.invalid);
      expect(result.reference, isNull);
    });

    test('rejects malformed shape, coordinate, shortcode, or hash', () {
      final invalidTags = <List<String>>[
        ['sticker', '30031:$_authorLower:cats', 'wave'],
        ['sticker', '30030:$_authorLower:cats', 'wave', _hashLower],
        ['sticker', '30031:not-hex:cats', 'wave', _hashLower],
        ['sticker', '30031:$_authorLower:bad:pack', 'wave', _hashLower],
        ['sticker', '30031:$_authorLower:cats', 'bad-shortcode', _hashLower],
        ['sticker', '30031:$_authorLower:cats', 'wave', 'not-a-hash'],
      ];

      for (final tag in invalidTags) {
        expect(
          parseStickerReference([tag]).status,
          StickerTagStatus.invalid,
          reason: 'expected invalid: $tag',
        );
      }
    });
  });

  test('cacheUrl targets the relay cache and encodes path segments', () {
    const reference = StickerReference(
      authorPubkey: _authorLower,
      packIdentifier: 'cats.fun',
      shortcode: 'Wave_1',
      sha256: _hashLower,
    );

    expect(
      reference.cacheUrl('https://relay.example/ignored?old=query#fragment'),
      'https://relay.example/media/sticker/$_authorLower/'
      'cats.fun/Wave_1/$_hashLower',
    );
    expect(reference.cacheUrl('not a URL'), isNull);
    expect(reference.cacheUrl('ftp://relay.example'), isNull);
  });
}
