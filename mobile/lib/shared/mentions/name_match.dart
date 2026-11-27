/// The one rule for matching a typed name to a person or agent name: the
/// portable mention rules, section 3 (buzz-app
/// `src/bundled/mentions/README.md`). Mentions, Space selection, query
/// admission and the recipient pickers all use it, so the same text finds
/// the same people, and `jose` finds `José`.
///
/// Normalization per code point: Unicode NFKD, the combining accents of the
/// "Combining Diacritical Marks" blocks removed, lowercase, and `ς` as `σ`.
/// Other marks stay, since in many scripts a mark makes a different letter
/// (`राम` is not `रम`, `ジョン` is not `ション`), and letters without a
/// decomposition do not fold (`ø`, `ł`, `æ`, `ß` and `ı` stay themselves).
/// This is deliberately narrower than platform diacritic-insensitive or full
/// case-folding comparisons, which the contract forbids.
library;

import 'package:unorm_dart/unorm_dart.dart' as unorm;

bool _isAccent(int rune) =>
    (rune >= 0x0300 && rune <= 0x036F) ||
    (rune >= 0x1AB0 && rune <= 0x1AFF) ||
    (rune >= 0x1DC0 && rune <= 0x1DFF) ||
    (rune >= 0x20D0 && rune <= 0x20FF) ||
    (rune >= 0xFE20 && rune <= 0xFE2F);

/// Folds one non-ASCII code point.
String _foldPoint(String char) {
  final buffer = StringBuffer();
  for (final rune in unorm.nfkd(char).runes) {
    if (!_isAccent(rune)) buffer.writeCharCode(rune);
  }
  return buffer.toString().toLowerCase().replaceAll('ς', 'σ');
}

/// Folding is the cost of matching, and the same names meet a new query on
/// every keystroke, so folded names stay folded. Long names are not kept.
final _folded = <String, String>{};
const _foldedLimit = 4096;
const _foldedLength = 128;

/// [text] without accents and case, as section 3 says. Keeps the spaces at
/// its ends, which query admission needs.
String foldName(String text) {
  final cached = _folded[text];
  if (cached != null) return cached;
  final buffer = StringBuffer();
  for (final rune in text.runes) {
    if (rune < 0x80) {
      // ASCII has no accents, decompositions or final sigma.
      buffer.writeCharCode(rune >= 0x41 && rune <= 0x5A ? rune + 0x20 : rune);
    } else {
      buffer.write(_foldPoint(String.fromCharCode(rune)));
    }
  }
  final folded = buffer.toString();
  if (text.length <= _foldedLength) {
    if (_folded.length >= _foldedLimit) _folded.clear();
    _folded[text] = folded;
  }
  return folded;
}

/// The form for comparing and ordering names: [foldName], trimmed.
String normalizeName(String text) => foldName(text).trim();

final _wordCharacter = RegExp(r'^[\p{L}\p{M}\p{N}]$', unicode: true);
final _whitespace = RegExp(r'\s', unicode: true);

/// A word is a run of letters, marks and digits; every other code point
/// separates words, so `jane` finds `Mary-Jane` and `brien` finds `O'Brien`.
bool isNameWordCharacter(int rune) {
  if (rune < 0x80) {
    return (rune >= 0x30 && rune <= 0x39) ||
        (rune >= 0x41 && rune <= 0x5A) ||
        (rune >= 0x61 && rune <= 0x7A);
  }
  return _wordCharacter.hasMatch(String.fromCharCode(rune));
}

bool _startsAt(List<int> chars, List<int> needle, int from) {
  if (from + needle.length > chars.length) return false;
  for (var i = 0; i < needle.length; i++) {
    if (chars[from + i] != needle[i]) return false;
  }
  return true;
}

/// Section 3 tier of [name] for a query whose [normalizeName] runes are
/// [needle], or null for no match. An empty query matches at tier 0.
int? foldedNameMatchTier(String name, List<int> needle) {
  if (needle.isEmpty) return 0;
  final chars = normalizeName(name).runes.toList();
  if (chars.length == needle.length && _startsAt(chars, needle, 0)) return 0;
  if (chars.length > needle.length && _startsAt(chars, needle, 0)) return 1;
  // A later word: the match starts where a word starts, and is that whole
  // word (tier 2) when it also ends where the word ends. A query with a
  // space matches only from the start of the name.
  if (needle.any((rune) => _whitespace.hasMatch(String.fromCharCode(rune)))) {
    return null;
  }
  var prefix = false;
  for (var from = 1; from < chars.length; from++) {
    if (!isNameWordCharacter(chars[from]) ||
        isNameWordCharacter(chars[from - 1]) ||
        !_startsAt(chars, needle, from)) {
      continue;
    }
    final stop = from + needle.length;
    if (stop >= chars.length || !isNameWordCharacter(chars[stop])) return 2;
    prefix = true;
  }
  return prefix ? 3 : null;
}

/// Section 3 tier of [name] for [query], or null for no match.
int? nameMatchTier(String name, String query) =>
    foldedNameMatchTier(name, normalizeName(query).runes.toList());

/// Whether [name] is longer than the normalized query [needle] and continues
/// it with a new word, as `Jose Luis` and `Jose-Luis` continue `jose` but
/// `Josefa` does not. Such a name keeps Space from selecting a shorter exact
/// name (section 5).
bool nameContinues(String name, String needle) {
  if (needle.isEmpty) return false;
  final whole = normalizeName(name).runes.toList();
  final query = needle.runes.toList();
  return whole.length > query.length &&
      _startsAt(whole, query, 0) &&
      !isNameWordCharacter(whole[query.length]);
}
