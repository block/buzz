import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

/// Message links are text inside their sentence, not widgets of their own, so
/// `find.text` cannot reach them. These find a link by its label instead.

/// The run of link [label] in [root]: its offset in the paragraph and the
/// style it is drawn in.
({int offset, TextStyle? style})? _linkRun(InlineSpan root, String label) {
  var offset = 0;
  ({int offset, TextStyle? style})? found;
  void visit(InlineSpan span, TextStyle? inherited) {
    if (found != null) return;
    final style = inherited?.merge(span.style) ?? span.style;
    if (span is! TextSpan) {
      offset += 1;
      return;
    }
    final text = span.text ?? '';
    if (span.recognizer != null && text == label) {
      found = (offset: offset, style: style);
      return;
    }
    offset += text.length;
    for (final child in span.children ?? const <InlineSpan>[]) {
      visit(child, style);
    }
  }

  visit(root, null);
  return found;
}

/// The paragraph holding link [label].
Finder findLinkParagraph(String label) => find.byWidgetPredicate(
  (widget) => widget is RichText && _linkRun(widget.text, label) != null,
  description: 'paragraph with link "$label"',
);

/// The style link [label] is drawn in.
TextStyle? linkStyle(WidgetTester tester, String label) => _linkRun(
  tester.widget<RichText>(findLinkParagraph(label)).text,
  label,
)!.style;

/// The centre of link [label], for tapping, holding or dragging from it.
Offset linkCenter(WidgetTester tester, String label) {
  final paragraph = tester.renderObject<RenderParagraph>(
    findLinkParagraph(label),
  );
  return _rangeCenter(
    paragraph,
    _linkRun(paragraph.text, label)!.offset,
    label,
  );
}

/// The centre of the first [text] in any paragraph, for touching plain text
/// beside a link.
Offset textCenter(WidgetTester tester, String text) {
  final paragraph = tester.renderObject<RenderParagraph>(
    find
        .byWidgetPredicate(
          (widget) =>
              widget is RichText && widget.text.toPlainText().contains(text),
        )
        .first,
  );
  final start = paragraph.text
      .toPlainText(includeSemanticsLabels: false)
      .indexOf(text);
  return _rangeCenter(paragraph, start, text);
}

Offset _rangeCenter(RenderParagraph paragraph, int start, String text) {
  final box = paragraph
      .getBoxesForSelection(
        TextSelection(baseOffset: start, extentOffset: start + text.length),
      )
      .first;
  return paragraph.localToGlobal(box.toRect().center);
}
