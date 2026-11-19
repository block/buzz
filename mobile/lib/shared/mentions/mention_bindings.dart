/// What continues a name directly after a label, as a pattern source: a
/// letter, mark or digit, or underscores leading to one, so `@Bobby` and
/// `@Bob_smith` hold no `Bob`. Anything else ends the name: punctuation, a
/// straight or curly apostrophe, a hyphen or a slash (`@Bob's`, `@Bob-bot`,
/// `@Bob/Alice`), and underscores that close Markdown emphasis (`_@Bob_`,
/// `__@Bob__`), which the composer's Italic button writes and sent messages
/// already hold. The composer's `mentionEndsAtBoundary`, the message scan
/// and the bubble's pill pattern all read this one rule.
const mentionNameContinuation = r'(?:[\p{L}\p{M}\p{N}]|_+[\p{L}\p{M}\p{N}])';

final _typedMentionEnd = RegExp(r'^(?:\|\||[\s,;.!?:)\]}*_]|$)');
final _markdownLinkLabel = RegExp(r'\[[^\]\n]*\]\(');

/// Whether a typed, unpicked name at `[start, end)` of [text] may notify by
/// name alone. A picked chip keeps its recipient before any suffix; a name
/// nobody picked must end as it always has, at whitespace, sentence
/// punctuation, a closing bracket or a Markdown delimiter, so the `@Alice` of
/// `@Alice@example.org` or `@Alice's` is prose. The label of a Markdown link
/// (`[@Alice](url)`) is link text, not a mention.
bool typedMentionNotifies(String text, int start, int end) {
  if (!_typedMentionEnd.hasMatch(text.substring(end))) return false;
  return !_markdownLinkLabel
      .allMatches(text)
      .any((link) => link.start < start && end < link.end - 1);
}

/// Longest literal ranges win, including labels containing another @ sign.
/// Recognition precedes eligibility: ambiguous labels still block shorter ones.
/// A label opens where a query does (portable mention rules, section 2): at
/// the start, after whitespace, `(`, `[` or `{`, or after a Markdown
/// delimiter, so a mention picked after a bracket keeps its recipient. It
/// ends at the composer's suffix boundary, [mentionNameContinuation].
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
        ? '(?! (?:[2-9]|[1-9][0-9]+)(?!$mentionNameContinuation))'
        : '';
    final pattern = RegExp(
      '(?:^|[\\s(\\[{]|[*_]{1,3}|\\|\\|)(@${RegExp.escape(label)})(?! \\([0-9a-f]{64}\\))$suffix(?!$mentionNameContinuation)',
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
