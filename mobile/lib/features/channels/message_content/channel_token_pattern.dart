/// Matches inline `#channel` tokens. The message body and its one-line
/// preview share it so they always agree on what is a channel token.
library;

/// Known [knownNames] match longest first, so `#eng-ios` wins over `#eng`.
/// A token can't start inside a word, path or URL (`a#b`, `x.y/#z`), and it
/// ends at whitespace, the end of a line, or punctuation that commonly follows
/// a word in prose: `#general's`, `"#general"`, `#general…`, `#general—`.
RegExp channelTokenPattern(Iterable<String> knownNames) {
  final names =
      knownNames
          .map((name) => name.trim())
          .where((name) => name.isNotEmpty)
          .toSet()
          .toList()
        ..sort((a, b) => b.length.compareTo(a.length));
  final alternatives = [...names.map(RegExp.escape), _genericName].join('|');
  return RegExp(
    '$_leading#(?:$alternatives)$_trailing',
    caseSensitive: false,
    multiLine: true,
  );
}

const _genericName = r'[A-Za-z0-9_][A-Za-z0-9_-]*';
const _leading = r'(?<![\w./:-])';
const _trailing =
    r'(?=$|[\s,;.!?:)\]}'
    "'\"\u2018\u2019\u201C\u201D\u00BB\u2026\u2013\u2014"
    r'])';
