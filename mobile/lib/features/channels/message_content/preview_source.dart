/// The leading part of [content] that a preview of [maxLines] lines can show.
///
/// A preview parses and lays out its whole source even though only
/// [maxLines] lines are painted, so a long message costs as much in a
/// two-line inbox row as in the timeline. Each unit kept here renders at
/// least one line, so units past the first `maxLines + 1` can never be
/// visible; the extra unit keeps the overflow that tells the paragraph it
/// was cut.
///
/// A unit is one source line, or a whole fenced code block, table or block
/// quote, which each render as one widget. Inline Markdown can span lines,
/// so the source is returned unchanged unless every delimiter that could
/// pair with one in the dropped tail is balanced in the kept part.
String markdownPreviewSource(String content, int maxLines) {
  final lines = content.split('\n');
  final prose = <String>[];
  var units = 0;
  var fenceOpen = false;
  String? group;
  for (var i = 0; i < lines.length; i++) {
    final line = lines[i];
    if (fenceOpen) {
      fenceOpen = !_fenceClose.hasMatch(line);
      continue;
    }
    final trimmed = line.trimLeft();
    if (trimmed.isEmpty) {
      group = null;
      continue;
    }
    final opensFence = _fenceOpen.hasMatch(line);
    // A run of backticks inside a line pairs in ways the units here do not
    // model, so such a message is parsed whole.
    if (!opensFence && line.contains('```')) return content;
    final lineGroup = trimmed.startsWith('|')
        ? '|'
        : trimmed.startsWith('>')
        ? '>'
        : null;
    if (lineGroup == null || lineGroup != group) {
      if (units == maxLines + 1) {
        final kept = lines.take(i).join('\n').trimRight();
        final outsideCode = _withoutCode(prose);
        return outsideCode != null &&
                _closesInline(kept) &&
                _closesInline(outsideCode)
            ? kept
            : content;
      }
      units++;
    }
    group = lineGroup;
    fenceOpen = opensFence;
    if (!opensFence) prose.add(line);
  }
  return content;
}

// gpt_markdown opens a fence only at the start of a line and closes it only
// at a line holding nothing but the closing backticks.
final _fenceOpen = RegExp(r'^ *```');
final _fenceClose = RegExp(r'^\s*```$');

/// The prose lines without their inline code spans, since delimiters inside
/// a span cannot pair with ones outside it, or null when a span cannot be
/// placed: a run of backticks, or a line with an odd number of them. With
/// single backticks only, gpt_markdown pairs them in order within a line, so
/// this is one linear pass over each line.
String? _withoutCode(List<String> prose) {
  final kept = <String>[];
  for (final line in prose) {
    if (line.contains('``')) return null;
    final parts = line.split('`');
    if (parts.length.isEven) return null;
    kept.add([for (var i = 0; i < parts.length; i += 2) parts[i]].join());
  }
  return kept.join('\n');
}

bool _closesInline(String text) {
  bool even(Pattern pattern) => pattern.allMatches(text).length.isEven;
  return even('**') &&
      even(RegExp(r'(?<!\*)\*(?!\*)')) &&
      even('~~') &&
      even(RegExp(r'(?<!`)`(?!`)')) &&
      _nests(text, '[', ']') &&
      _nests(text, r'\(', r'\)') &&
      _nests(text, '<u>', '</u>') &&
      !text.contains('【');
}

/// Whether every [open] in [text] is closed by a later [close] and no
/// [close] comes before its [open]. Each token is searched for once per
/// match, so this is linear in [text].
bool _nests(String text, String open, String close) {
  var depth = 0;
  var nextOpen = text.indexOf(open);
  var nextClose = text.indexOf(close);
  while (nextOpen >= 0 || nextClose >= 0) {
    if (nextClose < 0 || (nextOpen >= 0 && nextOpen < nextClose)) {
      depth++;
      nextOpen = text.indexOf(open, nextOpen + open.length);
    } else {
      if (--depth < 0) return false;
      nextClose = text.indexOf(close, nextClose + close.length);
    }
  }
  return depth == 0;
}
