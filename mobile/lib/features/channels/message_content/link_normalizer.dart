/// Escapes destination characters that the mobile Markdown renderer treats as
/// syntax. Apply the same conversion to metadata keys to preserve media types.
String normalizeMarkdownDestination(String url) =>
    url.replaceAll(' ', '%20').replaceAll('(', '%28').replaceAll(')', '%29');

const _markdownDelimiters = ['***', '___', '**', '__', '~~', '*', '_'];

final _autolinkPattern = RegExp(
  r'<((?:https?://|buzz://(?:message\?|join\?|channel/|(?:pr|issue|repo)\?))[^>]+)>',
);
final _bareLinkPattern = RegExp(
  r'(?<![(\]=])(?:https?://|buzz://(?:message\?|join\?|channel/|(?:pr|issue|repo)\?))[^\s)>\]]+',
);
final _trailingPunctuationPattern = RegExp(r'[.,!?:;]+$');
final _backtickRunPattern = RegExp(r'`+');

/// Converts supported Buzz and HTTP(S) autolinks and bare links into Markdown
/// links while leaving inline and fenced code untouched. Punctuation peeling
/// is limited to Buzz URLs so existing HTTP(S) destinations stay unchanged.
String normalizeBareLinks(String content) {
  final buffer = StringBuffer();
  var offset = 0;
  var proseStart = 0;
  var codeStart = 0;
  var inlineDelimiterLength = 0;
  var fenceDelimiterLength = 0;

  while (offset < content.length) {
    final run = _backtickRunPattern.matchAsPrefix(content, offset);
    if (run == null) {
      offset++;
      continue;
    }

    final runLength = run.end - run.start;
    if (fenceDelimiterLength > 0) {
      if (_isClosingFence(content, run.start, run.end, fenceDelimiterLength)) {
        buffer.write(content.substring(codeStart, run.end));
        fenceDelimiterLength = 0;
        proseStart = run.end;
      }
    } else if (inlineDelimiterLength > 0) {
      if (runLength == inlineDelimiterLength) {
        buffer.write(content.substring(codeStart, run.end));
        inlineDelimiterLength = 0;
        proseStart = run.end;
      }
    } else if (_hasInlineCloserOnLine(content, run.end, runLength) ||
        (!_isOpeningFence(content, run.start, runLength) &&
            _hasInlineCloser(content, run.end, runLength))) {
      buffer.write(
        _normalizeLinkSegment(content.substring(proseStart, run.start)),
      );
      codeStart = run.start;
      inlineDelimiterLength = runLength;
    } else if (_isOpeningFence(content, run.start, runLength)) {
      buffer.write(
        _normalizeLinkSegment(content.substring(proseStart, run.start)),
      );
      codeStart = run.start;
      fenceDelimiterLength = runLength;
    }

    offset = run.end;
  }

  if (inlineDelimiterLength > 0 || fenceDelimiterLength > 0) {
    buffer.write(content.substring(codeStart));
  } else {
    buffer.write(_normalizeLinkSegment(content.substring(proseStart)));
  }
  return buffer.toString();
}

bool _isOpeningFence(String content, int runStart, int runLength) {
  if (runLength < 3) return false;
  final lineStart = runStart == 0
      ? 0
      : content.lastIndexOf('\n', runStart - 1) + 1;
  final indentation = content.substring(lineStart, runStart);
  return indentation.length <= 3 && indentation.trim().isEmpty;
}

bool _isClosingFence(
  String content,
  int runStart,
  int runEnd,
  int openerLength,
) {
  if (runEnd - runStart < openerLength) return false;
  final lineStart = runStart == 0
      ? 0
      : content.lastIndexOf('\n', runStart - 1) + 1;
  final indentation = content.substring(lineStart, runStart);
  if (indentation.length > 3 || indentation.trim().isNotEmpty) return false;
  final newline = content.indexOf('\n', runEnd);
  final lineEnd = newline < 0 ? content.length : newline;
  return content.substring(runEnd, lineEnd).trim().isEmpty;
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
    result.write(_normalizeProseLinks(segment.substring(offset, match.start)));
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
  result.write(_normalizeProseLinks(segment.substring(offset)));
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

String _normalizeProseLinks(String segment) {
  var normalized = segment.replaceAllMapped(
    _autolinkPattern,
    (match) => '[${match[1]}](${match[1]})',
  );
  normalized = normalized.replaceAllMapped(
    _bareLinkPattern,
    (match) => _normalizeBareLink(normalized, match),
  );
  return normalized;
}

String _normalizeBareLink(String segment, Match match) {
  final matched = match[0]!;
  var url = matched;
  var trailing = '';
  final isBuzzUrl = matched.startsWith('buzz://');
  final start = match.start;

  if (isBuzzUrl) {
    final outsidePunctuation = _trailingPunctuationPattern.firstMatch(url);
    if (outsidePunctuation != null) {
      url = url.substring(0, outsidePunctuation.start);
      trailing = outsidePunctuation[0]!;
    }
  }

  var strippedDelimiter = true;
  while (strippedDelimiter) {
    strippedDelimiter = false;
    for (final delimiter in _markdownDelimiters) {
      if (url.endsWith(delimiter) &&
          _hasUnclosedMarkdownDelimiter(
            segment.substring(0, start),
            delimiter,
          )) {
        url = url.substring(0, url.length - delimiter.length);
        trailing = '$delimiter$trailing';
        strippedDelimiter = true;
        break;
      }
    }
  }

  if (isBuzzUrl) {
    final punctuation = _trailingPunctuationPattern.firstMatch(url);
    if (punctuation != null) {
      url = url.substring(0, punctuation.start);
      trailing = '${punctuation[0]}$trailing';
    }
  }

  // Preserve a URL already used as its own Markdown label. This covers both
  // converted autolinks and authored `[url](url)` links.
  if (start >= 1 && segment[start - 1] == '[') return matched;
  return '[$url]($url)$trailing';
}

bool _hasUnclosedMarkdownDelimiter(String prefix, String delimiter) {
  var open = false;
  var offset = 0;
  while (true) {
    final index = prefix.indexOf(delimiter, offset);
    if (index < 0) return open;
    final before = index == 0 ? null : prefix[index - 1];
    final afterIndex = index + delimiter.length;
    final after = afterIndex == prefix.length ? null : prefix[afterIndex];
    final canOpen =
        (after == null || after.trim().isNotEmpty) &&
        (before == null ||
            before.trim().isEmpty ||
            RegExp(r'[^\w]').hasMatch(before));
    if (open || canOpen) open = !open;
    offset = afterIndex;
  }
}
