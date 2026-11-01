import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import '../../../shared/theme/theme.dart';

/// Marks where an edited body draws its "(edited)" marker. A private-use
/// character, removed from authored content, so only the renderer adds it.
const editedSlot = '';

/// The visible text of the edited marker.
const editedMarkerText = '(edited)';

// Last lines that a following " (edited)" would join or turn into Markdown
// structure: indented code, a table row, a rule or setext underline, display
// math, or a bare list, quote, checkbox or radio marker. A heading takes its
// own line as on desktop, and so does a line ending in a link: links render
// as widget spans, so screen readers would read a joined marker first.
final _block = RegExp(
  r'^(?:\s{4}|\t)|^\s*\||\|\s*$|\$\$\s*$|\\\]\s*$'
  r'|^\s*(?:([-*_])(?:\s*\1){2,}|=+)\s*$'
  r'|^\s*#{1,6}(?:\s|$)'
  r'|[a-zA-Z][a-zA-Z0-9+.-]*://\S*\s*$|\]\([^)]*\)\s*$'
  r'|^\s*(?:[-*+]|>|\d+\.|\[[ xX]\]|\([ xX]\))\s*$',
);

/// [markdown] with the edited slot joined to its last line, or null when the
/// marker must take its own line instead. It joins prose (a paragraph, list
/// item or quote). Bodies with a code fence anywhere never get the
/// slot: gpt_markdown's fence rules differ from CommonMark, so the slot could
/// land inside code or open a new block.
String? withEditedSlot(String markdown) {
  final body = markdown.trimRight();
  if (body.isEmpty || body.contains('```')) return null;
  if (_block.hasMatch(body.split('\n').last)) return null;
  return '$body $editedSlot';
}

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
