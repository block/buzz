import 'package:buzz/shared/text/initial.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('uppercases an ordinary first letter', () {
    expect(avatarInitial('alice'), 'A');
    expect(avatarInitial('Bravo Beta'), 'B');
  });

  test('falls back to ? for an empty label', () {
    expect(avatarInitial(''), '?');
  });

  test('keeps a whole astral character instead of half a surrogate pair', () {
    // U+20000, CJK Extension B — an ordinary character in some names.
    const name = '\u{20000}\u{660E}';
    final initial = avatarInitial(name);
    expect(initial, '\u{20000}');
    expect(initial.runes.length, 1);
  });

  test('keeps an emoji whole', () {
    expect(avatarInitial('\u{1F389} party'), '\u{1F389}');
  });

  test('keeps a base letter together with its combining mark', () {
    // Devanagari: the vowel sign belongs to the consonant before it.
    expect(avatarInitial('\u{0928}\u{093F}\u{0932}'), '\u{0928}\u{093F}');
  });
}
