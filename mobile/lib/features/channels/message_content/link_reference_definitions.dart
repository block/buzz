/// A CommonMark link reference definition on a single line:
/// `[label]: destination "optional title"`, indented by at most three spaces.
final _definitionPattern = RegExp(
  r'''^ {0,3}\[((?:[^\[\]\\]|\\.){1,999})\]:[ \t]*(?:<[^<>\n]*>|\S+)'''
  r'''(?:[ \t]+(?:"[^"]*"|'[^']*'|\([^()]*\)))?[ \t]*$''',
);
final _fencePattern = RegExp(r'^ {0,3}(`{3,}|~{3,})');
final _whitespacePattern = RegExp(r'\s+');
final _leadingBlankLinesPattern = RegExp(r'^(?:[ \t]*\r?\n)+');

/// Removes link reference definitions that no reference in the message uses.
///
/// Desktop renders Markdown with a CommonMark parser, which drops a definition
/// such as `[label]: https://example.com` from the output. The mobile renderer
/// has no definitions and shows the line as text. An unused definition carries
/// nothing for the reader, so mobile drops it as well. A definition that a
/// `[label]` in the message points to stays: mobile cannot resolve the
/// reference, and the raw line is the only place its destination is visible.
///
/// Fenced code stays untouched, and a line that continues a paragraph is not a
/// definition, as in CommonMark.
String stripUnusedLinkReferenceDefinitions(String content) {
  if (!content.contains(']:')) return content;

  final lines = content.split('\n');
  final definitionLabels = <int, String>{};
  String? fenceDelimiter;
  var paragraphOpen = false;

  for (var index = 0; index < lines.length; index++) {
    final line = _withoutCarriageReturn(lines[index]);
    final fence = _fencePattern.firstMatch(line)?.group(1);
    if (fenceDelimiter != null) {
      if (fence != null &&
          fence[0] == fenceDelimiter[0] &&
          fence.length >= fenceDelimiter.length &&
          line.trim() == fence) {
        fenceDelimiter = null;
      }
      continue;
    }
    if (fence != null) {
      fenceDelimiter = fence;
      paragraphOpen = false;
      continue;
    }
    if (line.trim().isEmpty) {
      paragraphOpen = false;
      continue;
    }

    final definition = paragraphOpen
        ? null
        : _definitionPattern.firstMatch(line);
    final label = definition == null ? '' : _normalizeLabel(definition[1]!);
    if (label.isEmpty) {
      paragraphOpen = true;
    } else {
      definitionLabels[index] = label;
    }
  }
  if (definitionLabels.isEmpty) return content;

  final rest = _normalizeLabel(
    [
      for (var index = 0; index < lines.length; index++)
        if (!definitionLabels.containsKey(index)) lines[index],
    ].join('\n'),
  );
  final unused = {
    for (final entry in definitionLabels.entries)
      if (!rest.contains('[${entry.value}]')) entry.key,
  };
  if (unused.isEmpty) return content;

  final kept = [
    for (var index = 0; index < lines.length; index++)
      if (!unused.contains(index)) lines[index],
  ].join('\n');
  // A definition that opened the message was followed by a blank line;
  // without it the message would start with an empty paragraph.
  return unused.contains(0)
      ? kept.replaceFirst(_leadingBlankLinesPattern, '')
      : kept;
}

String _withoutCarriageReturn(String line) =>
    line.endsWith('\r') ? line.substring(0, line.length - 1) : line;

String _normalizeLabel(String label) =>
    label.trim().replaceAll(_whitespacePattern, ' ').toLowerCase();
