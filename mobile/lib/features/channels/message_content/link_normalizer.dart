/// Escapes destination characters that the mobile Markdown renderer treats as
/// syntax. Apply the same conversion to metadata keys to preserve media types.
String normalizeMarkdownDestination(String url) =>
    url.replaceAll(' ', '%20').replaceAll('(', '%28').replaceAll(')', '%29');

final _backtickRunPattern = RegExp(r'`+');
final _codeRunPattern = RegExp(r'`+|~{3,}');

/// Rewrites authored Markdown link and image destinations into the plain form
/// the gallery and attachment lookups match, leaving inline and fenced code
/// untouched. Bare URLs are left to gpt_markdown's autolinking.
String normalizeMarkdownLinks(String content) =>
    mapMarkdownProse(content, _normalizeLinkSegment);

/// Applies [transform] to each run of [content] outside inline and fenced
/// code, leaving code untouched. Fences may use backticks or tildes.
String mapMarkdownProse(String content, String Function(String) transform) {
  final buffer = StringBuffer();
  var offset = 0;
  var proseStart = 0;
  var codeStart = 0;
  var inlineDelimiterLength = 0;
  var fenceDelimiterLength = 0;
  var fenceChar = '';
  var fenceQuoteDepth = 0;

  while (offset < content.length) {
    // A fence opened inside a quote ends with that quote, closed or not.
    if (fenceDelimiterLength > 0 &&
        fenceQuoteDepth > 0 &&
        content[offset] == '\n' &&
        _lineQuoteDepth(content, offset + 1) < fenceQuoteDepth) {
      buffer.write(content.substring(codeStart, offset));
      fenceDelimiterLength = 0;
      proseStart = offset;
    }
    final run = _codeRunPattern.matchAsPrefix(content, offset);
    if (run == null) {
      offset++;
      continue;
    }

    final runLength = run.end - run.start;
    final runChar = content[run.start];
    if (fenceDelimiterLength > 0) {
      if (runChar == fenceChar &&
          _isClosingFence(
            content,
            run.start,
            run.end,
            fenceDelimiterLength,
            fenceQuoteDepth,
          )) {
        buffer.write(content.substring(codeStart, run.end));
        fenceDelimiterLength = 0;
        proseStart = run.end;
      }
    } else if (inlineDelimiterLength > 0) {
      if (runChar == '`' && runLength == inlineDelimiterLength) {
        buffer.write(content.substring(codeStart, run.end));
        inlineDelimiterLength = 0;
        proseStart = run.end;
      }
    } else if (runChar == '~') {
      // Tildes only fence code; shorter runs are strikethrough.
      if (_isOpeningFence(content, run.start, runLength)) {
        buffer.write(transform(content.substring(proseStart, run.start)));
        codeStart = run.start;
        fenceDelimiterLength = runLength;
        fenceChar = runChar;
        fenceQuoteDepth = _fenceQuoteDepth(content, run.start);
      }
    } else if (_isEscaped(content, run.start)) {
      // An escaped backtick is literal; the rest of its run may still open.
      offset = run.start + 1;
      continue;
    } else if (_hasInlineCloserOnLine(content, run.end, runLength) ||
        (!_isOpeningFence(content, run.start, runLength) &&
            _hasInlineCloser(content, run.end, runLength))) {
      buffer.write(transform(content.substring(proseStart, run.start)));
      codeStart = run.start;
      inlineDelimiterLength = runLength;
    } else if (_isOpeningFence(content, run.start, runLength)) {
      buffer.write(transform(content.substring(proseStart, run.start)));
      codeStart = run.start;
      fenceDelimiterLength = runLength;
      fenceChar = runChar;
      fenceQuoteDepth = _fenceQuoteDepth(content, run.start);
    }

    offset = run.end;
  }

  if (inlineDelimiterLength > 0 || fenceDelimiterLength > 0) {
    buffer.write(content.substring(codeStart));
  } else {
    buffer.write(transform(content.substring(proseStart)));
  }
  return buffer.toString();
}

/// Whether an odd number of backslashes precedes [index].
bool _isEscaped(String content, int index) {
  var backslashes = 0;
  while (index - backslashes > 0 && content[index - backslashes - 1] == r'\') {
    backslashes++;
  }
  return backslashes.isOdd;
}

bool _isOpeningFence(String content, int runStart, int runLength) =>
    runLength >= 3 && _fenceQuoteDepth(content, runStart) >= 0;

bool _isClosingFence(
  String content,
  int runStart,
  int runEnd,
  int openerLength,
  int openerQuoteDepth,
) {
  if (runEnd - runStart < openerLength) return false;
  // A fence closes only at the quote depth it opened at.
  if (_fenceQuoteDepth(content, runStart) != openerQuoteDepth) return false;
  // Stops at the first non-blank character, so each run scans only up to
  // the next one and a long line of runs stays linear.
  for (var i = runEnd; i < content.length; i++) {
    final char = content[i];
    if (char == '\n') return true;
    if (char.trim().isNotEmpty) return false;
  }
  return true;
}

/// The number of quote markers (`>`) before [runStart] on its line when only
/// blanks and markers precede it, or -1 when anything else does. This matches
/// where the renderer opens a fence: gpt_markdown re-parses quote bodies, and
/// list continuations are indented. The scan stops at the first other
/// character, so it never crosses an earlier run and a long line of runs
/// stays linear.
int _fenceQuoteDepth(String content, int runStart) {
  var depth = 0;
  for (var i = runStart - 1; i >= 0; i--) {
    final char = content[i];
    if (char == '\n') return depth;
    if (char == '>') {
      depth++;
    } else if (char.trim().isNotEmpty) {
      return -1;
    }
  }
  return depth;
}

/// The number of quote markers (`>`) in the blank-and-marker prefix of the
/// line starting at [lineStart].
int _lineQuoteDepth(String content, int lineStart) {
  var depth = 0;
  for (var i = lineStart; i < content.length; i++) {
    final char = content[i];
    if (char == '>') {
      depth++;
    } else if (char == '\n' || char.trim().isNotEmpty) {
      break;
    }
  }
  return depth;
}

bool _hasInlineCloserOnLine(String content, int start, int delimiterLength) {
  final newline = content.indexOf('\n', start);
  final lineEnd = newline < 0 ? content.length : newline;
  for (final run in _backtickRunPattern.allMatches(content, start)) {
    if (run.start >= lineEnd) return false;
    if (run.end - run.start == delimiterLength) return true;
  }
  return false;
}

bool _hasInlineCloser(String content, int start, int delimiterLength) {
  for (final run in _backtickRunPattern.allMatches(content, start)) {
    if (run.end - run.start == delimiterLength) return true;
  }
  return false;
}

// Numeric entities shield literal label punctuation from gpt_markdown's
// bracket-blind image/link matchers and its earlier \[...\] math pass.
String _normalizeMarkdownLabel(String label) => label.replaceAllMapped(
  RegExp(r'\\([\\\[\]])|[\[\]]'),
  (match) => '&#${(match[1] ?? match[0]!).codeUnitAt(0)};',
);

/// Restores the literal punctuation encoded for Markdown attachment labels.
String decodeMarkdownLabelSyntax(String label) => label.replaceAllMapped(
  RegExp(r'&#(91|92|93);'),
  (match) => String.fromCharCode(int.parse(match[1]!)),
);

// Scan each label character once, treating escaped brackets as label text.
// Nested brackets are balanced without restarting at every opening bracket.
({int start, int end})? _findMarkdownLinkStart(String segment, int offset) {
  final openers = <int>[];
  for (var cursor = offset; cursor < segment.length; cursor++) {
    final char = segment[cursor];
    if (char == r'\' &&
        cursor + 1 < segment.length &&
        segment[cursor + 1] != '\n') {
      cursor++;
      continue;
    }
    if (char == '\n') {
      openers.clear();
    } else if (char == '[') {
      openers.add(cursor);
    } else if (char == ']' && openers.isNotEmpty) {
      final opener = openers.removeLast();
      // A destination belongs to this matching opener even if an earlier
      // prose bracket never closed. Inner label brackets without a destination
      // still balance normally, preserving true nested labels.
      if (cursor + 1 < segment.length && segment[cursor + 1] == '(') {
        final start = opener > 0 && segment[opener - 1] == '!'
            ? opener - 1
            : opener;
        return (start: start, end: cursor + 2);
      }
    }
  }
  return null;
}

String _normalizeLinkSegment(String segment) {
  final result = StringBuffer();
  var offset = 0;
  while (offset < segment.length) {
    final match = _findMarkdownLinkStart(segment, offset);
    if (match == null) break;
    var cursor = match.end;
    if (cursor >= segment.length) break;
    final angled = segment[cursor] == '<';
    final start = angled ? ++cursor : cursor;
    var depth = 0;
    while (cursor < segment.length) {
      final char = segment[cursor];
      if (angled) {
        if (char == '>' || char == '\n' || char == '<') break;
      } else {
        if (char.trim().isEmpty) break;
        if (char == '(') depth++;
        if (char == ')') {
          if (depth == 0) break;
          depth--;
        }
      }
      cursor++;
    }
    final destinationEnd = cursor;
    var valid = cursor > start && cursor < segment.length;
    if (angled && valid) {
      valid = segment[cursor] == '>';
      if (valid) cursor++;
    }
    if (valid) {
      // gpt_markdown has no title parameter. Consume all Markdown title
      // delimiters and escaped delimiters before removing the title.
      final suffix = _scanLinkSuffix(segment, cursor);
      cursor = suffix.end;
      valid = suffix.valid;
    }
    result.write(segment.substring(offset, match.start));
    if (valid) {
      final destination = segment.substring(start, destinationEnd);
      final image = segment[match.start] == '!';
      final labelStart = match.start + (image ? 2 : 1);
      final label = _normalizeMarkdownLabel(
        segment.substring(labelStart, match.end - 2),
      );
      result.write(
        '${image ? '!' : ''}[$label](${normalizeMarkdownDestination(destination)})',
      );
    } else {
      // Preserve malformed source, and never scan its inspected suffix again.
      // This cursor advances on failure as well as success, bounding work even
      // for a relay-sized message made entirely of unterminated link openers.
      result.write(segment.substring(match.start, cursor));
    }
    offset = cursor;
  }
  result.write(segment.substring(offset));
  return result.toString();
}

({int end, bool valid}) _scanLinkSuffix(String segment, int cursor) {
  final start = cursor;
  while (cursor < segment.length && segment[cursor].trim().isEmpty) {
    cursor++;
  }
  if (cursor < segment.length && segment[cursor] == ')') {
    return (end: cursor + 1, valid: true);
  }
  if (cursor == start || cursor == segment.length) {
    return (end: cursor, valid: false);
  }
  final opener = segment[cursor];
  if (opener != '"' && opener != "'" && opener != '(') {
    return (end: cursor, valid: false);
  }
  final closer = opener == '(' ? ')' : opener;
  cursor++;
  while (cursor < segment.length) {
    final char = segment[cursor++];
    if (char == r'\' && cursor < segment.length) {
      cursor++;
    } else if (char == closer) {
      while (cursor < segment.length && segment[cursor].trim().isEmpty) {
        cursor++;
      }
      if (cursor < segment.length && segment[cursor] == ')') {
        return (end: cursor + 1, valid: true);
      }
      return (end: cursor, valid: false);
    } else if (opener == '(' && char == '(') {
      return (end: cursor, valid: false);
    }
  }
  return (end: cursor, valid: false);
}
