/// The single character shown in an avatar when no image is available.
library;

import 'package:characters/characters.dart';

/// Returns the first user-perceived character of [label], uppercased, or `?`
/// when [label] has none.
///
/// `label[0]` and `label.substring(0, 1)` return one UTF-16 code unit.
/// Anything outside the Basic Multilingual Plane — most emoji, and CJK
/// Extension B, which appears in ordinary Chinese and Japanese given names —
/// is stored as a surrogate pair, so those return half of one: not a
/// character, and drawn as `\u{FFFD}` in the avatar. They also throw on an
/// empty label; this returns `?`.
///
/// Grapheme clusters also keep a base letter together with its combining
/// marks, so a Devanagari or Burmese name keeps its vowel sign instead of
/// showing a bare consonant.
///
/// Mirrors desktop's `getInitials` (`desktop/src/shared/lib/initials.ts`) in
/// treating text as characters rather than code units.
String avatarInitial(String label) {
  final characters = label.characters;
  if (characters.isEmpty) return '?';
  return characters.first.toUpperCase();
}
