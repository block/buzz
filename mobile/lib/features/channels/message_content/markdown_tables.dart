/// [content] with a blank line before each pipe table that directly follows
/// a line of paragraph text.
///
/// In GFM, and on desktop, a table can interrupt a paragraph, so a table typed
/// right under its caption renders as a table. gpt_markdown starts a table
/// only on a fresh block and would show those rows as text. Fenced code is
/// left alone.
String separateMarkdownTables(String content) {
  if (!content.contains('|')) return content;
  final lines = content.split('\n');
  final out = <String>[];
  String? fence;
  for (var i = 0; i < lines.length; i++) {
    final line = lines[i];
    final trimmed = line.trimLeft();
    final marker = _fenceMarker.firstMatch(trimmed)?.group(1);
    if (marker != null) {
      if (fence == null) {
        fence = marker;
      } else if (marker[0] == fence[0] && marker.length >= fence.length) {
        fence = null;
      }
    } else if (fence == null &&
        i > 0 &&
        i + 1 < lines.length &&
        line == trimmed &&
        lines[i - 1].trim().isNotEmpty &&
        !lines[i - 1].startsWith('    ') &&
        _startsTable(line, lines[i + 1])) {
      out.add('');
    }
    out.add(line);
  }
  return out.join('\n');
}

/// Whether [header] and [delimiter] open a GFM table: the delimiter row is
/// only dashes, colons and pipes, with at least one pipe, and both rows have
/// the same cell count.
bool _startsTable(String header, String delimiter) {
  if (!header.contains('|') ||
      !delimiter.contains('|') ||
      !_delimiterRow.hasMatch(delimiter)) {
    return false;
  }
  return _cellCount(header) == _cellCount(delimiter);
}

int _cellCount(String row) {
  var cells = row.trim().split(_unescapedPipe);
  if (cells.first.trim().isEmpty) cells = cells.sublist(1);
  if (cells.isNotEmpty && cells.last.trim().isEmpty) {
    cells = cells.sublist(0, cells.length - 1);
  }
  return cells.length;
}

final _fenceMarker = RegExp(r'^(`{3,}|~{3,})');
// Each blank run has one place to go, so a long run followed by a stray
// character fails in linear time instead of retrying every split.
final _delimiterRow = RegExp(
  r'^ {0,3}\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*)*(?:\|\s*)?$',
);
final _unescapedPipe = RegExp(r'(?<!\\)\|');
