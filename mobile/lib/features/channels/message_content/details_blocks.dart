/// Collapsible sections in message markdown:
///
/// ```text
/// :::details Section title
/// Any markdown, collapsed until the reader opens it.
/// :::
/// ```
///
/// Mirrors desktop's `remarkDetails`: markers count at the start of a line
/// after up to three spaces (pasting into the composer can pull a marker into
/// the list item above it, indented as a continuation line; four spaces is
/// indented code), outside fenced code, in matched pairs and at most
/// [maxDetailsDepth] deep.
/// Anything else stays text. A marker line wrapped in emphasis as a whole
/// (`**:::details Title**`, `**:::**`, what the composer sends with bold
/// switched on) still counts. A no-break space counts as a space inside a
/// marker (the composer has sent one next to a bold title). A title may
/// carry inline formatting and start with `#`..`######` to render as a
/// heading.
library;

/// Deeper markers stay text, which bounds nesting on hostile input.
const maxDetailsDepth = 4;

final _openRe = RegExp(r'^:::details[ \t\u00a0]+(\S.*)$');
final _openPrefixRe = RegExp(r'^:::details[ \t\u00a0]+');
final _closeRe = RegExp(r'^:::[ \t\u00a0]*$');
final _wrappedRe = RegExp(r'^(\*\*|__|\*|_)(:::.*?)\1[ \t\u00a0]*$');
final _headingRe = RegExp(r'^(#{1,6})[ \t\u00a0]+');
final _indentRe = RegExp(r'^ {1,3}');
// Any indentation: a fence nested in a list item may sit deeper than the
// markers it contains. Reading indented code as a fence only keeps markers
// as text, the safe direction.
final _fenceOpenRe = RegExp(r'^[ \t]*(`{3,}|~{3,})(.*)$');
final _fenceCloseRe = RegExp(r'^[ \t]*(`{3,}|~{3,})[ \t]*$');

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
    if (raw.length > maxDetailsTitleLength) {
      // Never cut a surrogate pair in half.
      final end = _isHighSurrogate(raw.codeUnitAt(maxDetailsTitleLength - 1))
          ? maxDetailsTitleLength - 1
          : maxDetailsTitleLength;
      raw = raw.substring(0, end);
    }
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

/// The marker a line stands for, or null: unindented, `:::` for a closer,
/// one plain space after `:::details` and after a heading's hashes, and a
/// whole-line emphasis wrapper moved into the title
/// (`**:::details X**` → `:::details **X**`).
String? normalizeMarkerLine(String rawLine) {
  final line = rawLine.replaceFirst(_indentRe, '');
  var marker = line;
  var mark = '';
  if (!_openRe.hasMatch(line) && !_closeRe.hasMatch(line)) {
    final wrapped = _wrappedRe.firstMatch(line);
    if (wrapped != null) {
      mark = wrapped[1]!;
      marker = wrapped[2]!;
    }
  }
  if (_closeRe.hasMatch(marker)) return ':::';
  if (!_openRe.hasMatch(marker)) return null;
  final title = marker.replaceFirst(_openPrefixRe, '');
  final heading = _headingRe.firstMatch(title);
  final prefix = heading == null ? '' : '${heading[1]} ';
  final rest = title.substring(heading?[0]!.length ?? 0);
  return ':::details $prefix$mark$rest$mark';
}

/// Longest title the sanitizers read. Each title costs at most this squared,
/// so a hostile 64 KiB message stays bounded on the UI isolate; real titles
/// (including signed media URLs) fit well within it.
const maxDetailsTitleLength = 1000;

final _imageRe = RegExp(r'!\[([^\]\n]*)\]\((?:[^()\n]|\([^()\n]*\))*\)');
final _linkRe = RegExp(r'\[([^\]\n]*)\]\((?:[^()\n]|\([^()\n]*\))*\)');
final _autolinkRe = RegExp(r'<(https?://[^>\s]+)>');
final _emphasisRe = RegExp(r'(\*\*|__|~~|`|\*)(.+?)\1');
final _underscoreRe = RegExp(r'(?<!\w)_(.+?)_(?!\w)');

/// Images become their alt text, links their text, autolinks their URL,
/// as desktop's plain text does.
String _flattenLinks(String markdown) => markdown
    .replaceAllMapped(_imageRe, (m) => m[1]!)
    .replaceAllMapped(_linkRe, (m) => m[1]!)
    .replaceAllMapped(_autolinkRe, (m) => m[1]!);

/// Title markdown with links, images and autolinks flattened to text.
String _inlineTitleMarkdown(String markdown) => _flattenLinks(markdown).trim();

/// Shown when a title has no text of its own, e.g. `:::details [](url)`.
const detailsFallbackTitle = 'Details';

/// Best-effort markdown-to-text for titles, matching desktop's plain text:
/// images become alt text, link text kept, autolinks, code and emphasis
/// unwrapped.
String plainDetailsTitle(String markdown) {
  final title = _flattenLinks(markdown)
      .replaceAllMapped(_emphasisRe, (m) => m[2]!)
      .replaceAllMapped(_underscoreRe, (m) => m[1]!)
      .replaceAll(RegExp(r'\s+'), ' ')
      .trim();
  return title.isEmpty ? detailsFallbackTitle : title;
}

bool _isHighSurrogate(int unit) => unit >= 0xD800 && unit <= 0xDBFF;

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
