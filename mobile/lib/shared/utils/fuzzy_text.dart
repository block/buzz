/// Typo-tolerant word matching.
///
/// Ported from `desktop/src/shared/lib/fuzzyText.ts` so both clients accept
/// the same typing slips.
library;

final _wordSeparators = RegExp(r'[\s\-_./@]+');

/// Returns the Damerau-Levenshtein distance between two short strings.
///
/// Adjacent transpositions count as one edit, which covers common typing slips
/// such as `alcie` for `alice` without broadening ordinary substring matches.
int _editDistance(String left, String right) {
  final rows = left.length + 1;
  final columns = right.length + 1;
  final matrix = List.generate(rows, (_) => List.filled(columns, 0));

  for (var row = 0; row < rows; row++) {
    matrix[row][0] = row;
  }
  for (var column = 0; column < columns; column++) {
    matrix[0][column] = column;
  }

  for (var row = 1; row < rows; row++) {
    for (var column = 1; column < columns; column++) {
      final substitutionCost = left[row - 1] == right[column - 1] ? 0 : 1;
      var best = matrix[row - 1][column] + 1;
      if (matrix[row][column - 1] + 1 < best) {
        best = matrix[row][column - 1] + 1;
      }
      if (matrix[row - 1][column - 1] + substitutionCost < best) {
        best = matrix[row - 1][column - 1] + substitutionCost;
      }
      if (row > 1 &&
          column > 1 &&
          left[row - 1] == right[column - 2] &&
          left[row - 2] == right[column - 1] &&
          matrix[row - 2][column - 2] + 1 < best) {
        best = matrix[row - 2][column - 2] + 1;
      }
      matrix[row][column] = best;
    }
  }

  return matrix[left.length][right.length];
}

/// Whether [query] matches a word of [value], or a similarly-sized word
/// prefix, with one insertion, deletion, substitution, or adjacent
/// transposition allowed.
///
/// Four characters is the noise floor: shorter typo matches are too ambiguous.
bool hasTypoTolerantPrefixMatch(String value, String query) {
  final normalizedQuery = query.trim().toLowerCase();
  if (normalizedQuery.length < 4) return false;

  final words = value
      .toLowerCase()
      .split(_wordSeparators)
      .where((word) => word.isNotEmpty);
  for (final word in words) {
    final minimumLength = normalizedQuery.length - 1;
    final maximumLength = word.length < normalizedQuery.length + 1
        ? word.length
        : normalizedQuery.length + 1;

    for (var length = minimumLength; length <= maximumLength; length++) {
      if (_editDistance(normalizedQuery, word.substring(0, length)) <= 1) {
        return true;
      }
    }
  }

  return false;
}
