/// Collapsible sections in message markdown:
///
/// ```text
/// :::details Section title
/// Any markdown, collapsed until the reader opens it.
/// :::
/// ```
///
/// Mirrors desktop's `remarkDetails`: markers count only at column 0,
/// outside fenced code, and only in matched pairs. Anything else stays text.
library;

final _openRe = RegExp(r'^:::details[ \t]+(\S.*)$');
final _closeRe = RegExp(r'^:::[ \t]*$');
final _fenceRe = RegExp(r'^ {0,3}(`{3,}|~{3,})');

sealed class DetailsSegment {
  const DetailsSegment();
}

/// Markdown outside any section.
class DetailsText extends DetailsSegment {
  final String text;
  const DetailsText(this.text);
}

/// A top-level section. [body] may itself contain nested sections.
class DetailsBlock extends DetailsSegment {
  final String title;
  final String body;

  /// Stable within the message: occurrence index plus title, so repeated
  /// titles stay distinct and edits elsewhere do not reset the reader's
  /// open/closed choice.
  final String key;

  const DetailsBlock({
    required this.title,
    required this.body,
    required this.key,
  });
}

/// Splits [content] into text and top-level sections. Returns a single
/// [DetailsText] when the content has no complete section.
List<DetailsSegment> splitDetailsBlocks(String content) {
  if (!content.contains(':::details')) return [DetailsText(content)];
  final lines = content.split('\n');
  final pairs = _topLevelPairs(lines);
  if (pairs.isEmpty) return [DetailsText(content)];

  final segments = <DetailsSegment>[];
  final occurrences = <String, int>{};
  var cursor = 0;
  for (final (open, close) in pairs) {
    if (open > cursor) {
      segments.add(DetailsText(lines.sublist(cursor, open).join('\n')));
    }
    final title = _openRe.firstMatch(lines[open])!.group(1)!.trim();
    final occurrence = occurrences[title] ?? 0;
    occurrences[title] = occurrence + 1;
    segments.add(
      DetailsBlock(
        title: title,
        body: lines.sublist(open + 1, close).join('\n'),
        key: '$occurrence:$title',
      ),
    );
    cursor = close + 1;
  }
  if (cursor < lines.length) {
    segments.add(DetailsText(lines.sublist(cursor).join('\n')));
  }
  return segments;
}

/// Plain-text fallback for compact previews: markers dropped, titles kept.
String flattenDetailsBlocks(String content) {
  final segments = splitDetailsBlocks(content);
  if (segments.length == 1 && segments.single is DetailsText) return content;
  return segments
      .map(
        (segment) => switch (segment) {
          DetailsText(:final text) => text,
          DetailsBlock(:final title, :final body) =>
            '$title\n${flattenDetailsBlocks(body)}',
        },
      )
      .join('\n');
}

/// (open, close) line indices of matched pairs not nested in another matched
/// pair, in order. An unclosed opener stays text and does not hide the
/// sections after it.
List<(int, int)> _topLevelPairs(List<String> lines) {
  final pairs = <(int, int)>[];
  final open = <int>[];
  String? fence;
  for (var index = 0; index < lines.length; index++) {
    final line = lines[index];
    final fenceMatch = _fenceRe.firstMatch(line);
    if (fence != null) {
      final marker = fenceMatch?.group(1);
      if (marker != null &&
          marker[0] == fence[0] &&
          marker.length >= fence.length) {
        fence = null;
      }
      continue;
    }
    if (fenceMatch != null) {
      fence = fenceMatch.group(1);
      continue;
    }
    if (_openRe.hasMatch(line)) {
      open.add(index);
    } else if (_closeRe.hasMatch(line) && open.isNotEmpty) {
      final start = open.removeLast();
      // An enclosing pair closes later, so it replaces the pairs inside it.
      pairs.removeWhere((pair) => pair.$1 > start);
      pairs.add((start, index));
    }
  }
  return pairs;
}
