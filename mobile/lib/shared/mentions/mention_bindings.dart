final _selectionQualifier = RegExp(
  r' \([0-9a-f]{64}\)(?: (?:[2-9]|[1-9][0-9]+))?$',
  caseSensitive: false,
);

String _plainName(String label) =>
    label.replaceFirst(_selectionQualifier, '').toLowerCase();

/// Earlier plain selections of other identities sharing [name], with the
/// qualified keys they must be rewritten to once a namesake is selected.
Map<String, String> namesakeRequalifications(
  String name,
  String pubkey,
  Map<String, String> bindings,
) {
  final taken = bindings.keys.map((key) => key.toLowerCase()).toSet();
  return {
    for (final e in bindings.entries)
      if (e.key.toLowerCase() == name.toLowerCase() &&
          e.value.toLowerCase() != pubkey.toLowerCase() &&
          !taken.contains('${e.key} (${e.value})'.toLowerCase()))
        e.key: '${e.key} (${e.value.toLowerCase()})',
  };
}

/// Qualify plain mentions of [renames] keys, shifting [offset] with the text.
/// Each occurrence keeps its authored spelling; one inside [excluded] text,
/// such as code, stays as typed. [renamed] holds the keys with a qualified
/// occurrence, [kept] those with one left as typed.
({String text, int offset, Set<String> renamed, Set<String> kept})
renameMentions(
  String text,
  Iterable<String> labels,
  Map<String, String> renames,
  int offset, {
  bool Function(int start, int end)? excluded,
}) {
  var result = text;
  var shifted = offset;
  final renamed = <String>{};
  final kept = <String>{};
  for (final range in mentionOccurrences(text, labels).reversed) {
    final to = renames[range.label];
    // The query being replaced starts at [offset].
    if (to == null || range.start == offset) continue;
    if (excluded?.call(range.start, range.end) ?? false) {
      kept.add(range.label);
      continue;
    }
    final qualifier = to.substring(range.label.length);
    result = result.replaceRange(range.end, range.end, qualifier);
    renamed.add(range.label);
    if (range.end <= offset) shifted += qualifier.length;
  }
  return (text: result, offset: shifted, renamed: renamed, kept: kept);
}

/// Reserve an exact label without retargeting an earlier selection. Every
/// namesake is qualified: a qualified occurrence denies the plain name on render.
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
  final namesake = bindings.entries.any(
    (e) =>
        _plainName(e.key) == name.toLowerCase() &&
        e.value.toLowerCase() != pubkey.toLowerCase(),
  );
  if (!namesake && !conflicts(name)) return name;
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
