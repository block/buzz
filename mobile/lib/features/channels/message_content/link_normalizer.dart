const _markdownDelimiters = ['***', '___', '**', '__', '~~', '*', '_'];

final _autolinkPattern = RegExp(
  r'<((?:https?://|buzz://(?:message\?|join\?|channel/))[^>]+)>',
);
final _bareLinkPattern = RegExp(
  r'(?:https?://|buzz://(?:message\?|join\?|channel/))[^\s)>\]]+',
);
// Bare Buzz URLs stop at whitespace and Markdown's structural closers in the
// scanner above. Peel Unicode closing punctuation, final quotes, and terminal
// punctuation that can be adjacent in prose. ASCII apostrophe and quotation
// mark are not Unicode closing punctuation, so include them explicitly. This
// is intentionally Buzz-only so existing HTTP(S) normalization is unchanged.
final _trailingProseDelimiterPattern = RegExp(
  r'''(?:['"]|[\p{Pe}\p{Pf}]|\p{Terminal_Punctuation})+$''',
  unicode: true,
);
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

String _normalizeLinkSegment(String segment) {
  var normalized = segment.replaceAllMapped(_autolinkPattern, (match) {
    final url = match[1]!;
    // An angle-bracket URL immediately after a Markdown label is that link's
    // destination, not an autolink. Buzz schemes entered this pass in this
    // feature; leave existing HTTP(S) behavior unchanged.
    if (url.startsWith('buzz://') &&
        match.start >= 2 &&
        segment.substring(match.start - 2, match.start) == '](') {
      return url;
    }
    return '[$url]($url)';
  });
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

  // Existing Markdown destinations and imeta attributes already own the URL.
  // Check the preceding character explicitly instead of using RegExp
  // lookbehind so this scanner remains portable to older runtimes.
  if (start > 0) {
    final previous = segment[start - 1];
    if (previous == '(' ||
        previous == '\\' ||
        previous == ']' ||
        previous == '=' ||
        (previous == '<' &&
            start >= 3 &&
            segment.substring(start - 3, start) == '](<')) {
      return matched;
    }
  }

  if (isBuzzUrl) {
    final outsideDelimiters = _trailingProseDelimiterPattern.firstMatch(url);
    if (outsideDelimiters != null) {
      url = url.substring(0, outsideDelimiters.start);
      trailing = outsideDelimiters[0]!;
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
    final delimiters = _trailingProseDelimiterPattern.firstMatch(url);
    if (delimiters != null) {
      url = url.substring(0, delimiters.start);
      trailing = '${delimiters[0]}$trailing';
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
