/// Channel name matching for search.
///
/// Ported from buzz-app `src/features/search/match.ts` (`matchName`), so a
/// query finds the same channels, in the same order, as desktop search.
library;

/// How well a name matches typed text, best first. Ranks are whole numbers,
/// so callers can order them and add fractional tie-breaks.
abstract final class MatchRank {
  static const exact = 0;
  static const prefix = 1;

  /// The text starts a later word.
  static const wordStart = 2;
  static const substring = 3;

  /// Each letter continues a run that began at a word start (`tlp` in
  /// `team-launch-planning`).
  static const wordRuns = 4;

  /// The letters appear in order.
  static const inOrder = 5;
}

final _wordChar = RegExp(r'[\p{L}\p{N}]', unicode: true);
final _spaces = RegExp(r'\s+');

bool _isWordChar(String? char) => char != null && _wordChar.hasMatch(char);

List<String> _codePoints(String value) =>
    value.runes.map(String.fromCharCode).toList();

/// Returns how well [label] matches [needle] as a [MatchRank], or null for no
/// match. [needle] must already be lowercased.
int? matchRank(String label, String needle) {
  final text = label.toLowerCase();
  if (text == needle) return MatchRank.exact;

  var substring = false;
  for (
    var at = text.indexOf(needle);
    at >= 0;
    at = text.indexOf(needle, at + 1)
  ) {
    if (at == 0) return MatchRank.prefix;
    if (!_isWordChar(text[at - 1])) return MatchRank.wordStart;
    substring = true;
  }
  if (substring) return MatchRank.substring;

  // Spaces in typed text only separate words; they need not match.
  final letters = _codePoints(needle.replaceAll(_spaces, ''));
  if (letters.isEmpty) return null;
  final chars = _codePoints(text);

  // Letters in order, the loosest rank. Without them nothing below matches,
  // so a name that misses skips the word-run search.
  var found = 0;
  for (final char in chars) {
    if (found < letters.length && char == letters[found]) found++;
  }
  if (found < letters.length) return null;

  // Can letters[i..] match chars[j..] with every run starting at a word start?
  // `inRun` means the previous letter matched chars[j - 1].
  final memo = <int, bool>{};
  late final bool Function(int, int, bool) wordRuns;
  bool takes(int i, int j, bool inRun) =>
      chars[j] == letters[i] &&
      (inRun || !_isWordChar(j > 0 ? chars[j - 1] : null)) &&
      wordRuns(i + 1, j + 1, true);
  wordRuns = (i, j, inRun) {
    if (i == letters.length) return true;
    if (j == chars.length) return false;
    final key = (i * (chars.length + 1) + j) * 2 + (inRun ? 1 : 0);
    final known = memo[key];
    if (known != null) return known;
    final result = takes(i, j, inRun) || wordRuns(i, j + 1, false);
    memo[key] = result;
    return result;
  };
  if (wordRuns(0, 0, false)) return MatchRank.wordRuns;
  return MatchRank.inOrder;
}
