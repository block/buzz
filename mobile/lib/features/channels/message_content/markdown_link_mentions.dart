import 'package:gpt_markdown/gpt_markdown.dart';

import '../../../shared/mentions/bound_mentions.dart';
import 'link_normalizer.dart';

const _firstMark = 0xE000;
const _markCount = 0xF8FF - _firstMark + 1;
final _privateUse = RegExp('[-]');

/// Which of [mentions], `@Label` ranges of [text] in text order, sit inside
/// a Markdown link as a message bubble reads one: a link's label, destination
/// or title, an image, an autolink, a bare URL or a link reference
/// definition. The result holds their indexes in [mentions].
///
/// Link text is not a mention. The bubble draws no pill inside a label, and
/// an identity link written there would nest a link inside a link and break
/// it, so a mention inside a link is never a chip and is saved as typed.
///
/// The answer comes from the bubble's own pipeline, [normalizeBareLinks] and
/// then the Markdown parser, not from a pattern, so escapes, code, nesting
/// and reference links are read as the bubble reads them. The parser's tree
/// has no source offsets, so each label is first replaced by one private-use
/// character after its `@`, a letter-like character no syntax uses, and the
/// tree says where each ended up. A mention counts as outside a link only
/// when its mark is found in text outside every link, so anything the tree
/// does not show as plain text stays as typed.
///
/// The cost follows the length of [text] and never the channel's roster.
/// Text with neither `]` nor `://` holds no link and is not parsed.
Set<int> mentionsInsideMarkdownLinks(String text, List<MentionRange> mentions) {
  if (mentions.isEmpty || !(text.contains(']') || text.contains('://'))) {
    return const {};
  }
  final source = text.replaceAll(_privateUse, '�');
  final inside = <int>{};
  for (var first = 0; first < mentions.length; first += _markCount) {
    final count = (mentions.length - first).clamp(0, _markCount);
    final marked = StringBuffer();
    var offset = 0;
    for (var i = 0; i < count; i++) {
      final mention = mentions[first + i];
      marked
        ..write(source.substring(offset, mention.start + 1))
        ..writeCharCode(_firstMark + i);
      offset = mention.end;
    }
    marked.write(source.substring(offset));
    final plain = <int>{};
    _collectPlainMarks(
      Plusparse.parse(normalizeBareLinks(marked.toString())).children,
      plain,
    );
    for (var i = 0; i < count; i++) {
      if (!plain.contains(i)) inside.add(first + i);
    }
  }
  return inside;
}

void _marksIn(String text, Set<int> plain) {
  for (var i = 0; i < text.length; i++) {
    final unit = text.codeUnitAt(i) - _firstMark;
    if (unit >= 0 && unit < _markCount) plain.add(unit);
  }
}

/// Adds the marks in the text of [nodes] that is not part of a link, an
/// image, a citation or a footnote reference.
void _collectPlainMarks(List<MdNode> nodes, Set<int> plain) {
  for (final node in nodes) {
    switch (node) {
      case MdText(:final text) || MdInlineCode(:final text):
        _marksIn(text, plain);
      case MdCodeBlock(:final code):
        _marksIn(code, plain);
      case MdInlineLatex(:final tex) || MdBlockLatex(:final tex):
        _marksIn(tex, plain);
      case MdCustomBlock(:final body):
        _marksIn(body, plain);
      case MdHeading(:final children) ||
          MdParagraph(:final children) ||
          MdBlockQuote(:final children) ||
          MdCheckbox(:final children) ||
          MdRadio(:final children) ||
          MdBold(:final children) ||
          MdItalic(:final children) ||
          MdStrike(:final children) ||
          MdUnderline(:final children):
        _collectPlainMarks(children, plain);
      case MdUnorderedList(:final items) || MdOrderedList(:final items):
        for (final item in items) {
          _collectPlainMarks(item.children, plain);
        }
      case MdTable(:final header, :final rows):
        for (final row in [header, ...rows]) {
          for (final cell in row.cells) {
            _collectPlainMarks(cell.content, plain);
          }
        }
      case MdFootnoteDefinitions(:final footnotes):
        for (final footnote in footnotes) {
          _collectPlainMarks(footnote.children, plain);
        }
      case MdLink() ||
          MdImage() ||
          MdSourceTag() ||
          MdFootnoteReference() ||
          MdHorizontalRule() ||
          MdLineBreak():
        break;
    }
  }
}
