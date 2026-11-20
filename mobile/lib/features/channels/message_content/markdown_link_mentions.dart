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
/// The cost follows the length of [text] and never the channel's roster.
/// Text with no backtick, `~~~`, `]` or `://` holds neither code nor a link
/// and is not parsed.
Set<int> mentionsInsideMarkdownLinks(
  String text,
  List<MentionRange> mentions, {
  int keep = 1,
}) {
  if (mentions.isEmpty ||
      !(text.contains(']') ||
          text.contains('://') ||
          text.contains('`') ||
          text.contains('~~~'))) {
    return const {};
  }
  final source = text.replaceAll(_privateUse, '\uFFFD');
  // The ranges that hold the same text share a mark.
  final sameText = <String, List<int>>{};
  for (final (i, mention) in mentions.indexed) {
    (sameText[source.substring(mention.start, mention.end)] ??= []).add(i);
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
    for (var i = 0; i < mentions.length; i++)
      if (!prose.contains(i)) i,
  };
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
