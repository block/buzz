import 'dart:math';

import 'package:gpt_markdown/gpt_markdown.dart';

import '../../../shared/mentions/bound_mentions.dart';
import 'link_normalizer.dart';

const _firstMark = 0xE000;
const _markCount = 0xF8FF - _firstMark + 1;
final _privateUse = RegExp(r'[\uE000-\uF8FF]', unicode: true);

// The letters after a mark: their case numbers the ranges sharing it.
const _zero = 0x61;
const _one = 0x41;

/// Which of [mentions], `@Label` ranges of [text] in text order, a message
/// bubble does not read as prose: those inside inline code or a code block
/// (single or longer backtick runs, backtick or `~~~` fences of any length,
/// an unclosed fence), or inside a Markdown link: a link's label, destination
/// or title, an image, an autolink, a bare URL, a reference and its link
/// reference definition. The result holds their indexes in [mentions].
///
/// Code and link text are not mentions. The bubble draws no pill there, and
/// an identity link written there would show as raw text in code or nest a
/// link inside a link and break it, so such a mention is never a chip and is
/// saved as typed.
///
/// The answer comes from the bubble's own pipeline, [normalizeBareLinks] and
/// then the Markdown parser, not from a pattern, so escapes, code, nesting
/// and reference links are read as the bubble reads them. The parser's tree
/// has no source offsets, so each range is first replaced, after its first
/// [keep] characters (the `@` of a mention; 0 for a whole range), by a
/// token no syntax uses, and the tree says where each ended up. A token is
/// one private-use character, letter-like, shared by the ranges that hold
/// exactly the same text, followed by letters whose case alone tells those
/// ranges apart. The bubble lifts each mention out before parsing in the same
/// way, with one stand-in per distinct text, and the parser matches a
/// reference to its definition without regard to case. So `[@Bob]` still
/// finds `[@Bob]: <url>` and both are read as the link they form, while
/// `[@Bob]` beside `[@bob]: <url>` is no link here, as in the bubble. A range
/// counts as prose only when its token is found in text outside all code and
/// every link, so anything the
/// tree does not show as plain text stays as typed.
///
/// With [portable], the answer also covers two places this bubble draws a
/// pill but standard Markdown readers, buzz-app among them, do not read as
/// prose, so that text an edit rewrites is rewritten only where every reader
/// agrees it is a mention:
///
/// - a reference whose definition differs only in case (`[@Bob]` with
///   `[@bob]: <url>`), which those readers pair into a link. Ranges whose
///   text differs only in case then share a mark.
/// - an indented code block ([_indentedCode]), which this parser reads as a
///   paragraph.
///
/// The cost follows the length of [text] and never the channel's roster.
/// Text with no backtick, `~~~`, `]` or `://` holds neither code nor a link
/// and is not parsed.
Set<int> mentionsInsideMarkdownLinks(
  String text,
  List<MentionRange> mentions, {
  int keep = 1,
  bool portable = false,
}) {
  if (mentions.isEmpty) return const {};
  final indented = portable ? _indentedCode(text) : const <MentionRange>[];
  final code = {
    for (final (i, mention) in mentions.indexed)
      if (indented.any((c) => c.start <= mention.start && mention.end <= c.end))
        i,
  };
  if (!(text.contains(']') ||
      text.contains('://') ||
      text.contains('`') ||
      text.contains('~~~'))) {
    return code;
  }
  final source = text.replaceAll(_privateUse, '\uFFFD');
  // The ranges that hold the same text share a mark.
  final sameText = <String, List<int>>{};
  for (final (i, mention) in mentions.indexed) {
    final label = source.substring(mention.start, mention.end);
    (sameText[portable ? label.toLowerCase() : label] ??= []).add(i);
  }
  final groups = sameText.values.toList();
  final prose = <int>{};
  for (var first = 0; first < groups.length; first += _markCount) {
    final batch = groups.skip(first).take(_markCount).toList();
    final bits = (batch.map((g) => g.length).reduce(max) - 1).bitLength;
    final tokens = <int, String>{};
    for (var mark = 0; mark < batch.length; mark++) {
      for (var n = 0; n < batch[mark].length; n++) {
        tokens[batch[mark][n]] = String.fromCharCodes([
          _firstMark + mark,
          for (var bit = 0; bit < bits; bit++)
            (n >> bit) & 1 == 1 ? _one : _zero,
        ]);
      }
    }
    final marked = StringBuffer();
    var offset = 0;
    for (var i = 0; i < mentions.length; i++) {
      final token = tokens[i];
      if (token == null) continue;
      marked
        ..write(source.substring(offset, mentions[i].start + keep))
        ..write(token);
      offset = mentions[i].end;
    }
    marked.write(source.substring(offset));
    _collectPlainText(
      Plusparse.parse(normalizeBareLinks(marked.toString())).children,
      (plain) {
        for (var i = 0; i < plain.length; i++) {
          final mark = plain.codeUnitAt(i) - _firstMark;
          if (mark < 0 || mark >= batch.length || i + bits >= plain.length) {
            continue;
          }
          var n = 0;
          for (var bit = 0; bit < bits; bit++) {
            final unit = plain.codeUnitAt(i + 1 + bit);
            if (unit == _one) {
              n |= 1 << bit;
            } else if (unit != _zero) {
              n = batch[mark].length;
              break;
            }
          }
          if (n < batch[mark].length) prose.add(batch[mark][n]);
        }
      },
    );
  }
  return {
    ...code,
    for (var i = 0; i < mentions.length; i++)
      if (!prose.contains(i)) i,
  };
}

final _quoteMarker = RegExp(r'^ {0,3}> ?');
final _fenceOpen = RegExp(r'^(`{3,}|~{3,})');
final _atxHeading = RegExp(r'^#{1,6}(?:[ \t]|$)');
final _thematicBreak = RegExp(r'^([-*_])[ \t]*(?:\1[ \t]*){2,}$');
final _setextUnderline = RegExp(r'^(?:=+|-+)[ \t]*$');
final _listMarker = RegExp(r'^(?:[-+*]|(\d{1,9})[.)])(?=[ \t]|$)');

/// The width of the leading whitespace of [line] from [from], in columns
/// with a tab stop every four, and the offset where it ends.
({int columns, int end}) _indent(String line, [int from = 0]) {
  var columns = 0;
  var i = from;
  for (; i < line.length; i++) {
    final unit = line.codeUnitAt(i);
    if (unit == 0x20) {
      columns++;
    } else if (unit == 0x09) {
      columns += 4 - columns % 4;
    } else {
      break;
    }
  }
  return (columns: columns, end: i);
}

/// The lines of [text] that CommonMark reads as an indented code block, as
/// whole-line ranges. A line is one when it is indented four or more columns
/// (a tab counts to the next multiple of four) past its container and does
/// not continue a paragraph:
///
/// - A line directly under paragraph text is that paragraph's continuation,
///   however far it is indented, also across a block quote or list item
///   that the paragraph sits in (a lazy continuation). Only a blank line, a
///   heading, a thematic break, a setext underline, a fence or earlier
///   indented code ends the paragraph first.
/// - Inside a list item the indent is counted from the item's content
///   column, so the item's own paragraphs are not code and a line four
///   columns deeper is; nested items stack. An item ends at a later line
///   indented less than its content.
/// - Inside a block quote the indent is counted after the `>` markers.
/// - Lines inside a backtick or `~~~` fence are left to the parser.
///
/// HTML blocks and tables are read as paragraph text.
List<MentionRange> _indentedCode(String text) {
  final code = <MentionRange>[];
  final items = <int>[];
  var paragraph = false;
  var quotes = 0;
  String? fence;
  var start = 0;
  while (start <= text.length) {
    var end = text.indexOf('\n', start);
    if (end < 0) end = text.length;
    final range = (start: start, end: end);
    var line = text.substring(start, end);
    start = end + 1;

    var depth = 0;
    for (var m = _quoteMarker.firstMatch(line); m != null; depth++) {
      line = line.substring(m.end);
      m = _quoteMarker.firstMatch(line);
    }
    var (:columns, end: at) = _indent(line);
    var content = line.substring(at);

    if (fence != null) {
      if (content.startsWith(fence) &&
          content.replaceAll(fence[0], '').trim().isEmpty) {
        fence = null;
      }
      continue;
    }
    if (content.isEmpty) {
      paragraph = false;
      quotes = depth;
      continue;
    }

    // The column this line's indent is counted from: the content of the
    // innermost list item it is indented enough to belong to.
    int base() => items.lastWhere((item) => item <= columns, orElse: () => 0);
    bool startsBlock() =>
        columns - base() < 4 &&
        (_fenceOpen.hasMatch(content) ||
            _atxHeading.hasMatch(content) ||
            _thematicBreak.hasMatch(content) ||
            _listMarker.hasMatch(content));
    if (paragraph) {
      if (depth == quotes && _setextUnderline.hasMatch(content)) {
        paragraph = false;
        continue;
      }
      // A list that would open under a paragraph must start with a
      // non-empty item, numbered 1 when ordered.
      final marker = _listMarker.firstMatch(content);
      final opensList =
          marker != null &&
          (items.isNotEmpty ||
              (content.length > marker.end &&
                  content.substring(marker.end).trim().isNotEmpty &&
                  (marker[1] == null || int.parse(marker[1]!) == 1)));
      if (depth <= quotes && (!startsBlock() || marker != null && !opensList)) {
        continue;
      }
    }
    if (depth != quotes) items.clear();
    quotes = depth;
    paragraph = false;
    items.removeWhere((item) => item > columns);

    while (true) {
      final indent = columns - (items.lastOrNull ?? 0);
      if (indent >= 4) {
        code.add(range);
        break;
      }
      if (_fenceOpen.firstMatch(content) case final open?) {
        fence = open[1];
        break;
      }
      if (_atxHeading.hasMatch(content) || _thematicBreak.hasMatch(content)) {
        break;
      }
      final marker = _listMarker.firstMatch(content);
      if (marker == null) {
        paragraph = true;
        break;
      }
      // The item's content starts one to four columns after its marker;
      // more than that is one column and then indented code.
      final after = _indent(content, marker.end);
      final blank = after.end == content.length;
      final gap = blank || after.columns > 4 ? 1 : after.columns;
      columns += marker.end + gap;
      items.add(columns);
      if (blank) break;
      content = content.substring(after.end);
      columns += after.columns - gap;
    }
  }
  return code;
}

/// Hands [plain] the text of [nodes] that is not code and not part of a
/// link, an image, a citation or a footnote reference.
void _collectPlainText(List<MdNode> nodes, void Function(String) plain) {
  for (final node in nodes) {
    switch (node) {
      case MdText(:final text):
        plain(text);
      case MdInlineLatex(:final tex) || MdBlockLatex(:final tex):
        plain(tex);
      case MdCustomBlock(:final body):
        plain(body);
      case MdHeading(:final children) ||
          MdParagraph(:final children) ||
          MdBlockQuote(:final children) ||
          MdCheckbox(:final children) ||
          MdRadio(:final children) ||
          MdBold(:final children) ||
          MdItalic(:final children) ||
          MdStrike(:final children) ||
          MdUnderline(:final children):
        _collectPlainText(children, plain);
      case MdUnorderedList(:final items) || MdOrderedList(:final items):
        for (final item in items) {
          _collectPlainText(item.children, plain);
        }
      case MdTable(:final header, :final rows):
        for (final row in [header, ...rows]) {
          for (final cell in row.cells) {
            _collectPlainText(cell.content, plain);
          }
        }
      case MdFootnoteDefinitions(:final footnotes):
        for (final footnote in footnotes) {
          _collectPlainText(footnote.children, plain);
        }
      case MdInlineCode() ||
          MdCodeBlock() ||
          MdLink() ||
          MdImage() ||
          MdSourceTag() ||
          MdFootnoteReference() ||
          MdHorizontalRule() ||
          MdLineBreak():
        break;
    }
  }
}
