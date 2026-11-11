import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import '../../../shared/theme/theme.dart';

/// Marks where an edited body draws its "(edited)" marker. A private-use
/// character. A body that already has one is drawn as written, and its
/// marker takes its own line.
const editedSlot = '';

/// The visible text of the edited marker.
const editedMarkerText = '(edited)';

// Last lines that a following " (edited)" would join or turn into Markdown
// structure: indented code, a table row, a rule or setext underline, `$$`
// display math, or a bare list, quote, checkbox or radio marker. A heading
// takes its own line as on desktop. Escapes are already shielded here, so
// `\]` is text, not math. Each pattern starts at the line start or at a
// fixed character, so no run of text is rescanned and a match stays linear in
// the line, whatever an author puts in it.
final _block = RegExp(
  r'^(?:\s{4}|\t)|^\s*\||\|\s*$|\$\$\s*$'
  r'|^\s*(?:([-*_])(?:\s*\1){2,}|=+)\s*$'
  r'|^\s*#{1,6}(?:\s|$)'
  r'|^\s*(?:[-*+]|>|\d+\.|\[[ xX]\]|\([ xX]\))\s*$',
);

/// [markdown] with the edited slot joined to its last line, or null when the
/// marker must take its own line instead. It joins prose (a paragraph, list
/// item or quote). Bodies with a code fence anywhere never get the
/// slot: gpt_markdown's fence rules differ from CommonMark, so the slot could
/// land inside code or open a new block. Bodies with an authored slot never
/// get one either, so that character is not drawn as the marker.
String? withEditedSlot(String markdown) {
  final body = markdown.trimRight();
  if (body.isEmpty || body.contains('```') || body.contains(editedSlot)) {
    return null;
  }
  final last = body.substring(body.lastIndexOf('\n') + 1);
  if (_block.hasMatch(last) || _endsInLink(last)) return null;
  return '$body $editedSlot';
}

/// Whether [line] ends in a URL or a Markdown link. Links render as widget
/// spans, so screen readers would read a joined marker before the link. A
/// scan, not a regular expression: an unanchored one restarts at every
/// character and takes quadratic time on a long authored line.
bool _endsInLink(String line) {
  final text = line.trimRight();
  // A link: `](`, then no `)` up to the final one.
  if (text.length > 2 && text.endsWith(')')) {
    final previous = text.lastIndexOf(')', text.length - 2);
    if (text.substring(previous + 1, text.length - 1).contains('](')) {
      return true;
    }
  }
  // A URL: `://` in the last word, after a scheme that has a letter.
  final word = text.substring(text.lastIndexOf(_space) + 1);
  for (var i = word.indexOf('://'); i > 0; i = word.indexOf('://', i + 1)) {
    for (var j = i - 1; j >= 0 && _schemeChar.hasMatch(word[j]); j--) {
      if (_letter.hasMatch(word[j])) return true;
    }
  }
  return false;
}

final _space = RegExp(r'\s');
final _schemeChar = RegExp(r'[a-zA-Z0-9+.-]');
final _letter = RegExp(r'[a-zA-Z]');

/// Metadata after an edited body: the accent ink on your own bubble, else
/// the muted metadata colour.
TextStyle editedMarkerStyle(BuildContext context, {required bool own}) =>
    // Pin the traits a heading or an open <u> would pass down to the slot.
    messageTimestampTextStyle.copyWith(
      fontStyle: FontStyle.normal,
      decoration: TextDecoration.none,
      color: own
          ? ChatColors.of(context).foreground
          : context.colors.onSurfaceVariant,
    );

/// [markdown] drawn by [render] with the edited marker after its last line:
/// joined to prose, else on its own line inside the same text bubble. [slot]
/// says whether the message's renderer draws [editedSlot] as the marker; it
/// does not when any part of the message has an authored slot.
Widget markedEditedBody(
  String markdown,
  Widget Function(String) render,
  TextStyle style, {
  required bool slot,
}) {
  final slotted = slot ? withEditedSlot(markdown) : null;
  if (slotted != null) return render(slotted);
  return Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    mainAxisSize: MainAxisSize.min,
    children: [
      render(markdown),
      Text(editedMarkerText, style: style),
    ],
  );
}

/// [child], then the edited marker below it, outside every text bubble.
Widget withEditedMarkerBelow(Widget child, {required bool end}) => Column(
  crossAxisAlignment: end ? CrossAxisAlignment.end : CrossAxisAlignment.start,
  mainAxisSize: MainAxisSize.min,
  children: [child, const EditedMarkerBelow()],
);

/// The edited marker after trailing media, outside every text bubble.
class EditedMarkerBelow extends StatelessWidget {
  /// Creates the marker.
  const EditedMarkerBelow({super.key});

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
    child: Text(
      editedMarkerText,
      style: editedMarkerStyle(context, own: false),
    ),
  );
}

/// Draws [editedSlot] inside Markdown as the edited marker.
class EditedMarkerMd extends InlineMd {
  /// The marker's text style.
  final TextStyle style;

  /// Creates the component with its resolved [style].
  EditedMarkerMd(this.style);

  @override
  RegExp get exp => RegExp(editedSlot);

  @override
  InlineSpan span(
    BuildContext context,
    String text,
    GptMarkdownConfig config,
  ) => TextSpan(text: editedMarkerText, style: style);
}
