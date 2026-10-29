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
        return _closesInline(kept) &&
                !outsideCode.contains('`') &&
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
final _inlineCode = RegExp(r'`(?!`)(.+?)(?<!`)`(?!`)');

/// The prose lines without their inline code spans, since delimiters inside
/// a span cannot pair with ones outside it. A backtick left over means a span
/// this cannot place, and the caller keeps the whole message.
String _withoutCode(List<String> prose) =>
    prose.map((line) => line.replaceAll(_inlineCode, '')).join('\n');

bool _closesInline(String text) {
  bool even(Pattern pattern) => pattern.allMatches(text).length.isEven;
  int count(String token) => token.allMatches(text).length;
  return even('**') &&
      even(RegExp(r'(?<!\*)\*(?!\*)')) &&
      even('~~') &&
      even(RegExp(r'(?<!`)`(?!`)')) &&
      count('[') == count(']') &&
      count(r'\(') == count(r'\)') &&
      count('<u>') == count('</u>') &&
      !text.contains('【');
}
