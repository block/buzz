/// A letter, mark, digit or underscore directly after a label continues the
/// name, so `@Bobby` holds no `Bob`. Anything else ends it: punctuation, a
/// straight or curly apostrophe, a hyphen or a slash (`@Bob's`, `@Bob-bot`,
/// `@Bob/Alice`). Same rule as the composer's `mentionEndsAtBoundary`.
const _nameCharacter = r'[\p{L}\p{M}\p{N}_]';

/// Longest literal ranges win, including labels containing another @ sign.
/// Recognition precedes eligibility: ambiguous labels still block shorter ones.
/// A label opens where a query does (portable mention rules, section 2): at
/// the start, after whitespace, `(`, `[` or `{`, or after a Markdown
/// delimiter, so a mention picked after a bracket keeps its recipient. It
/// ends at the composer's suffix boundary, [_nameCharacter].
List<({int start, int end, String label})> mentionOccurrences(
  String text,
  Iterable<String> labels,
) {
  final matches = <int, ({int start, int end, String label})>{};
  for (final label in labels) {
    if (label.isEmpty) continue;
    // A qualifier and reservation suffix belong to the literal even when no
    // candidate binds it. Never fall back to a shorter, different recipient.
    final suffix =
        RegExp(r' \([0-9a-f]{64}\)$', caseSensitive: false).hasMatch(label)
        ? '(?! (?:[2-9]|[1-9][0-9]+)(?!$_nameCharacter))'
        : '';
    final pattern = RegExp(
      '(?:^|[\\s(\\[{]|[*_]{1,3}|\\|\\|)(@${RegExp.escape(label)})(?! \\([0-9a-f]{64}\\))$suffix(?!$_nameCharacter)',
      caseSensitive: false,
      unicode: true,
    );
    for (final match in pattern.allMatches(text)) {
      final start = match.end - match.group(1)!.length;
      if (matches[start] == null || matches[start]!.end < match.end) {
        matches[start] = (start: start, end: match.end, label: label);
      }
    }
  }
  final result = <({int start, int end, String label})>[];
  for (final match
      in matches.values.toList()..sort((a, b) => a.start.compareTo(b.start))) {
    if (result.isEmpty || match.start >= result.last.end) result.add(match);
  }
  return result;
}

/// Bind tagged qualifiers and ordinary aliases, never historical namesakes.
Map<String, Set<String>> renderedMentionBindings(
  String content,
  Map<String, String> names, [
  Iterable<String> signedPubkeys = const [],
]) {
  final bindings = <String, Set<String>>{};
  void add(String label, String key) =>
      (bindings[label.toLowerCase()] ??= {}).add(key.toLowerCase());
  for (final entry in names.entries) {
    add(entry.value, entry.key);
    add(entry.value.split(RegExp(r'\s+')).first, entry.key);
  }
  final keys = signedPubkeys.map((key) => key.toLowerCase()).toSet();
  for (final match in RegExp(
    r'@([^@\r\n]+) \(([0-9a-f]{64})\)(?: ((?:[1-9][0-9]+|[2-9])))?',
    caseSensitive: false,
  ).allMatches(content)) {
    final key = match.group(2)!.toLowerCase();
    final label = match.group(0)!.substring(1).toLowerCase();
    if (!mentionOccurrences(content, [
      label,
    ]).any((range) => range.start == match.start)) {
      continue;
    }
    // Text can deny historical aliases without a profile, never authorize keys.
    bindings[label] = keys.contains(key) ? {key} : <String>{};
    // Recipient tags/current names cannot establish the historical plain owner.
    bindings[match.group(1)!.toLowerCase()] = <String>{};
  }
  return bindings;
}
