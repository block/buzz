const WORD_SEPARATORS = /[\s\-_./@]+/;

/**
 * Return the Damerau-Levenshtein distance between two short strings.
 * Adjacent transpositions count as one edit, which covers common typing slips
 * such as `alcie` for `alice` without broadening ordinary substring matches.
 */
function editDistance(left: string, right: string) {
  const rows = left.length + 1;
  const columns = right.length + 1;
  const matrix = Array.from({ length: rows }, () =>
    Array<number>(columns).fill(0),
  );

  for (let row = 0; row < rows; row += 1) matrix[row][0] = row;
  for (let column = 0; column < columns; column += 1) {
    matrix[0][column] = column;
  }

  for (let row = 1; row < rows; row += 1) {
    for (let column = 1; column < columns; column += 1) {
      const substitutionCost = left[row - 1] === right[column - 1] ? 0 : 1;
      matrix[row][column] = Math.min(
        matrix[row - 1][column] + 1,
        matrix[row][column - 1] + 1,
        matrix[row - 1][column - 1] + substitutionCost,
      );

      if (
        row > 1 &&
        column > 1 &&
        left[row - 1] === right[column - 2] &&
        left[row - 2] === right[column - 1]
      ) {
        matrix[row][column] = Math.min(
          matrix[row][column],
          matrix[row - 2][column - 2] + 1,
        );
      }
    }
  }

  return matrix[left.length][right.length];
}

/**
 * Match a query against complete words or similarly-sized word prefixes with
 * one insertion, deletion, substitution, or adjacent transposition allowed.
 * Four characters is the noise floor: shorter typo matches are too ambiguous.
 */
export function hasTypoTolerantPrefixMatch(value: string, query: string) {
  const normalizedQuery = query.trim().toLowerCase();
  if (normalizedQuery.length < 4) return false;

  const words = value.toLowerCase().split(WORD_SEPARATORS).filter(Boolean);
  for (const word of words) {
    const minimumLength = Math.max(1, normalizedQuery.length - 1);
    const maximumLength = Math.min(word.length, normalizedQuery.length + 1);

    for (let length = minimumLength; length <= maximumLength; length += 1) {
      if (editDistance(normalizedQuery, word.slice(0, length)) <= 1) {
        return true;
      }
    }
  }

  return false;
}
