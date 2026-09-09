/// Reserve an exact label without retargeting an earlier selection.
String selectedMentionLabel(
  String name,
  String pubkey,
  Map<String, String> bindings,
) {
  final normalized = {
    for (final e in bindings.entries)
      e.key.toLowerCase(): e.value.toLowerCase(),
  };
  bool conflicts(String label) =>
      normalized.containsKey(label.toLowerCase()) &&
      normalized[label.toLowerCase()] != pubkey.toLowerCase();
  if (!conflicts(name)) return name;
  final qualified = '$name (${pubkey.toLowerCase()})';
  var label = qualified;
  for (var suffix = 2; conflicts(label); suffix++) {
    label = '$qualified $suffix';
  }
  return label;
}

/// Longest literal ranges win, including labels containing another @ sign.
/// Recognition precedes eligibility: ambiguous labels still block shorter ones.
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
        ? r'(?! (?:[2-9]|[1-9][0-9]+)(?=[\s,;.!?:)\]}*_]|$))'
        : '';
    final pattern = RegExp(
      '(?:^|\\s|[*_]{1,3}|\\|\\|)(@${RegExp.escape(label)})(?! \\([0-9a-f]{64}\\))$suffix(?=\\|\\||[\\s,;.!?:)\\]}*_]|\$)',
      caseSensitive: false,
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

/// Reconstruct display bindings only from event-tagged identities, never text
/// alone. Qualified tagged labels survive profile renames; ambiguous aliases
/// remain unbound. Mirrors Desktop's resolveMentionProps.
Map<String, Set<String>> renderedMentionBindings(
  String content,
  Map<String, String> names,
) {
  final bindings = <String, Set<String>>{};
  void add(String label, String key) =>
      (bindings[label.toLowerCase()] ??= {}).add(key.toLowerCase());
  for (final entry in names.entries) {
    add(entry.value, entry.key);
    add(entry.value.split(RegExp(r'\s+')).first, entry.key);
  }
  final keys = names.keys.map((key) => key.toLowerCase()).toSet();
  final qualified = <({String label, String base, String key})>[];
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
    // Untagged qualified literals are recognition blockers, not bindings.
    bindings.putIfAbsent(label, () => <String>{});
    if (!keys.contains(key)) continue;
    qualified.add((
      label: label,
      base: match.group(1)!.toLowerCase(),
      key: key,
    ));
    add(label, key);
    bindings.putIfAbsent(match.group(1)!.toLowerCase(), () => <String>{});
  }
  final winning = mentionOccurrences(
    content,
    bindings.keys,
  ).map((range) => range.label).toSet();
  final claimed = qualified.where((q) => winning.contains(q.label)).toList();
  final bases = claimed.map((q) => q.base).where(winning.contains).toSet();
  final remaining = keys.difference(claimed.map((q) => q.key).toSet());
  // Ordinary sibling mentions claim their own identities before recovering a
  // historical namesake. Current aliases must not retarget that namesake.
  for (final label in winning.difference(bases)) {
    final candidates = bindings[label]!;
    if (candidates.length == 1) remaining.removeAll(candidates);
  }
  for (final base in bases) {
    // Multiple historical labels or leftover tagged keys cannot be paired
    // deterministically. Leave them as non-clickable references, not guesses.
    bindings[base] = bases.length == 1 ? remaining.toSet() : <String>{};
  }
  return bindings;
}
