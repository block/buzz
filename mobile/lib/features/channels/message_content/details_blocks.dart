/// Collapsible sections in message markdown:
///
/// ```text
/// :::details Section title
/// Any markdown, collapsed until the reader opens it.
/// :::
/// ```
///
/// Mirrors desktop's `remarkDetails`: markers count only at column 0,
/// outside fenced code, in matched pairs and at most [maxDetailsDepth] deep.
/// Anything else stays text. A marker line wrapped in emphasis as a whole
/// (`**:::details Title**`, `**:::**`, what the composer sends with bold
/// switched on) still counts. A title may carry inline formatting and start
/// with `#`..`######` to render as a heading.
library;

/// Deeper markers stay text, which bounds nesting on hostile input.
const maxDetailsDepth = 4;

final _openRe = RegExp(r'^:::details[ \t]+(\S.*)$');
final _closeRe = RegExp(r'^:::[ \t]*$');
final _wrappedRe = RegExp(r'^(\*\*|__|\*|_)(:::.*?)\1[ \t]*$');
final _headingRe = RegExp(r'^(#{1,6})[ \t]+');
final _fenceOpenRe = RegExp(r'^ {0,3}(`{3,}|~{3,})(.*)$');
final _fenceCloseRe = RegExp(r'^ {0,3}(`{3,}|~{3,})[ \t]*$');

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
  /// Plain text: the accessible label and the section key.
  final String title;

  /// Title with inline formatting only; links and images flattened, so the
  /// disclosure control holds no other controls.
  final String titleMarkdown;

  /// Heading level 1-6, or 0 for a plain title.
  final int level;
  final String body;

  /// Stable within the message: occurrence index plus title, so repeated
  /// titles stay distinct and edits elsewhere do not reset the reader's
  /// open/closed choice.
  final String key;

  const DetailsBlock({
    required this.title,
    required this.titleMarkdown,
    required this.level,
    required this.body,
    required this.key,
  });
}

/// Splits [content] into text and top-level sections. Returns a single
/// [DetailsText] when the content has no complete section. Sections nested
/// in a body are allowed [depth] levels fewer.
List<DetailsSegment> splitDetailsBlocks(String content, {int depth = 0}) {
  if (!content.contains(':::details')) return [DetailsText(content)];
  final lines = content.split('\n');
  final pairs = _topLevelPairs(lines, maxDetailsDepth - depth);
  if (pairs.isEmpty) return [DetailsText(content)];

  final segments = <DetailsSegment>[];
  final occurrences = <String, int>{};
  var cursor = 0;
  for (final (open, close) in pairs) {
    if (open > cursor) {
      segments.add(DetailsText(lines.sublist(cursor, open).join('\n')));
    }
    var raw = _openRe
        .firstMatch(normalizeMarkerLine(_stripCr(lines[open]))!)!
        .group(1)!;
    final heading = _headingRe.firstMatch(raw);
    if (heading != null) raw = raw.substring(heading.end);
    final title = plainDetailsTitle(raw);
    final titleMarkdown = title == detailsFallbackTitle
        ? title
        : _inlineTitleMarkdown(raw);
    final occurrence = occurrences[title] ?? 0;
    occurrences[title] = occurrence + 1;
    segments.add(
      DetailsBlock(
        title: title,
        titleMarkdown: titleMarkdown,
        level: heading == null ? 0 : heading[1]!.length,
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
/// Iterative over the bounded nesting depth.
String flattenDetailsBlocks(String content) {
  var text = content;
  for (var depth = 0; depth < maxDetailsDepth; depth++) {
    final segments = splitDetailsBlocks(text, depth: depth);
    if (segments.length == 1 && segments.single is DetailsText) break;
    text = segments
        .map(
          (segment) => switch (segment) {
            DetailsText(:final text) => text,
            DetailsBlock(:final titleMarkdown, :final level, :final body) =>
              '${level > 0 ? '${'#' * level} ' : ''}$titleMarkdown\n$body',
          },
        )
        .join('\n');
  }
  return text;
}

/// The marker a line stands for, with a whole-line emphasis wrapper moved
/// into the title (`**:::details X**` → `:::details **X**`), or null.
String? normalizeMarkerLine(String line) {
  if (_openRe.hasMatch(line) || _closeRe.hasMatch(line)) return line;
  final wrapped = _wrappedRe.firstMatch(line);
  if (wrapped == null) return null;
  final mark = wrapped[1]!;
  final inner = wrapped[2]!;
  if (_closeRe.hasMatch(inner)) return ':::';
  if (!_openRe.hasMatch(inner)) return null;
  final title = inner.replaceFirst(RegExp(r'^:::details[ \t]+'), '');
  final heading = _headingRe.firstMatch(title)?[0] ?? '';
  return ':::details $heading$mark${title.substring(heading.length)}$mark';
}

/// Title markdown with links, images and autolinks flattened to text.
String _inlineTitleMarkdown(String markdown) => markdown
    .replaceAll(RegExp(r'!\[[^\]]*\]\([^)]*\)'), '')
    .replaceAllMapped(RegExp(r'\[([^\]]*)\]\([^)]*\)'), (m) => m[1]!)
    .replaceAllMapped(RegExp(r'<(https?://[^>\s]+)>'), (m) => m[1]!)
    .trim();

/// Shown when a title has no text of its own, e.g. `:::details [](url)`.
const detailsFallbackTitle = 'Details';

/// Best-effort markdown-to-text for titles, matching desktop's plain text:
/// images dropped, link text kept, autolinks, code and emphasis unwrapped.
String plainDetailsTitle(String markdown) {
  final title = markdown
      .replaceAll(RegExp(r'!\[[^\]]*\]\([^)]*\)'), '')
      .replaceAllMapped(RegExp(r'\[([^\]]*)\]\([^)]*\)'), (m) => m[1]!)
      .replaceAllMapped(RegExp(r'<(https?://[^>\s]+)>'), (m) => m[1]!)
      .replaceAllMapped(RegExp(r'(\*\*|__|~~|`|\*)(.+?)\1'), (m) => m[2]!)
      .replaceAllMapped(RegExp(r'(?<!\w)_(.+?)_(?!\w)'), (m) => m[1]!)
      .replaceAll(RegExp(r'\s+'), ' ')
      .trim();
  return title.isEmpty ? detailsFallbackTitle : title;
}

String _stripCr(String line) =>
    line.endsWith('\r') ? line.substring(0, line.length - 1) : line;

String? _fenceOpen(String line) {
  final match = _fenceOpenRe.firstMatch(line);
  if (match == null) return null;
  final fence = match[1]!;
  // CommonMark: a backtick fence's info string may not contain a backtick.
  return fence.startsWith('`') && match[2]!.contains('`') ? null : fence;
}

bool _closesFence(String line, String fence) {
  final marker = _fenceCloseRe.firstMatch(line)?[1];
  return marker != null &&
      marker[0] == fence[0] &&
      marker.length >= fence.length;
}

/// (open, close) line indices of matched pairs not nested in another matched
/// pair, in order. An unclosed opener stays text and does not hide the
/// sections after it; openers past [maxDepth] stay text with their closers.
List<(int, int)> _topLevelPairs(List<String> lines, int maxDepth) {
  final pairs = <(int, int)>[];
  // Opener line index, or -1 for an opener past the depth limit.
  final open = <int>[];
  var depth = 0;
  String? fence;
  for (var index = 0; index < lines.length; index++) {
    final line = _stripCr(lines[index]);
    if (fence != null) {
      if (_closesFence(line, fence)) fence = null;
      continue;
    }
    fence = _fenceOpen(line);
    if (fence != null) continue;
    final marker = normalizeMarkerLine(line);
    if (marker == null) continue;
    if (_openRe.hasMatch(marker)) {
      final allowed = depth < maxDepth;
      open.add(allowed ? index : -1);
      if (allowed) depth++;
    } else if (_closeRe.hasMatch(marker) && open.isNotEmpty) {
      final start = open.removeLast();
      if (start < 0) continue;
      depth--;
      // Pairs inside this one closed last, so they sit at the end.
      while (pairs.isNotEmpty && pairs.last.$1 > start) {
        pairs.removeLast();
      }
      pairs.add((start, index));
    }
  }
  return pairs;
}
