/// Fuzzy channel matching for search.
///
/// Ported from `desktop/src/features/channels/lib/channelSearchScore.ts` so a
/// query finds the same channels, in the same order, on both clients. Exact
/// and prefix matches keep their stable score bands; looser passes come after:
///
///  1. word-boundary tokens (split on space/`-`/`_`/`.`/`/`), so a later word
///     of a multi-word name matches,
///  2. separator-free substring, so `releasenotes` finds `release-notes`,
///  3. an in-order subsequence check, so `reln` finds `release-notes`, and
///  4. a one-edit word-prefix fallback, so `genral` finds `general`.
///
/// Lower score is a better match. `null` means no match.
library;

import '../../shared/utils/fuzzy_text.dart';

final _wordSeparators = RegExp(r'[\s\-_./]+');

const _scoreExact = 0;
const _scorePrefix = 1;
const _scoreWordExact = 2;
const _scoreWordPrefix = 3;
const _scoreSubstring = 4;
const _scoreCollapsedSeparators = 5;
const _scoreSubsequence = 6;
const _scoreTypo = 7;
const _scoreDescription = 8;

/// Strips separators so `release-notes` and `releasenotes` compare equal.
String _collapseSeparators(String value) =>
    value.replaceAll(_wordSeparators, '');

/// Whether every character of [query] appears in [text] in order.
bool _isSubsequence(String query, String text) {
  if (query.isEmpty) return true;
  var queryIndex = 0;
  for (var index = 0; index < text.length; index++) {
    if (text[index] == query[queryIndex]) {
      queryIndex++;
      if (queryIndex == query.length) return true;
    }
  }
  return false;
}

/// Scores how well [name] matches [lowerQuery], or returns null for no match.
///
/// [lowerQuery] must already be lowercased and trimmed.
int? scoreChannelName(String name, String lowerQuery) {
  if (lowerQuery.isEmpty) return _scoreExact;

  final lower = name.toLowerCase();

  if (lower == lowerQuery) return _scoreExact;
  if (lower.startsWith(lowerQuery)) return _scorePrefix;

  final words = lower.split(_wordSeparators).where((word) => word.isNotEmpty);
  if (words.any((word) => word == lowerQuery)) return _scoreWordExact;
  if (words.any((word) => word.startsWith(lowerQuery))) {
    return _scoreWordPrefix;
  }

  if (lower.contains(lowerQuery)) return _scoreSubstring;

  final collapsedName = _collapseSeparators(lower);
  final collapsedQuery = _collapseSeparators(lowerQuery);
  if (collapsedQuery.isNotEmpty && collapsedName.contains(collapsedQuery)) {
    return _scoreCollapsedSeparators;
  }

  // One-character queries would match almost everything as a subsequence.
  if (collapsedQuery.length >= 2 && _isSubsequence(collapsedQuery, lower)) {
    return _scoreSubsequence;
  }

  if (hasTypoTolerantPrefixMatch(lower, lowerQuery)) return _scoreTypo;

  return null;
}

/// Scores a channel's [name] and [description] against [lowerQuery].
///
/// Description matches are plain substring and always rank below any name
/// match, so supplementary text never crowds out channels named for the query.
int? scoreChannelMatch({
  required String name,
  required String description,
  required String lowerQuery,
}) {
  if (lowerQuery.isEmpty) return _scoreExact;

  final nameScore = scoreChannelName(name, lowerQuery);
  if (nameScore != null) return nameScore;

  if (description.toLowerCase().contains(lowerQuery)) {
    return _scoreDescription;
  }

  return null;
}
