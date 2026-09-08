import 'package:flutter/material.dart';
import 'package:gpt_markdown/custom_widgets/markdown_config.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import '../theme/grid.dart';

/// Called when a task checkbox is tapped, with its zero-based position in the
/// message and the state the reader asked for.
typedef TaskToggleCallback = void Function(int taskIndex, bool checked);

/// Renders a GFM task-list item (`- [ ] revisar el relay`) as a checkbox.
///
/// This exists because `gpt_markdown` cannot render the syntax the rest of the
/// product speaks. Its own `CheckBoxMd` matches `[ ] text` with **no** bullet,
/// while `UnOrderedList` — which sits earlier in the component list and
/// therefore wins — matches `- anything`. The upshot on an unmodified package
/// is that every task list written on desktop arrives here as a plain bullet
/// with a literal `[ ]` in the text. Registering this component *before*
/// `UnOrderedList` is what makes the two platforms agree.
///
/// The marker pattern is deliberately identical to `task_markers.dart`
/// (bracket followed by whitespace or end-of-line), so the checkbox a reader
/// taps and the marker the toggle rewrites are always the same one.
class GfmTaskListMd extends BlockMd {
  GfmTaskListMd({this.onToggle});

  /// `null` leaves the checkboxes rendered but inert — the state for a message
  /// the reader may not edit, and for every read-only surface.
  final TaskToggleCallback? onToggle;

  /// Ordinal handed to [onToggle]. `gpt_markdown` builds blocks in document
  /// order within one pass, and a fresh instance is constructed on every
  /// render (see `message_content.dart`), so this counts from zero each time
  /// the message is drawn.
  int _seen = 0;

  @override
  String get expString =>
      r'(?:[-*+]|\d{1,9}[.)])[ \t]+\[([ xX])\](?=[ \t]|$)[ \t]*([^\n]*)$';

  @override
  Widget build(BuildContext context, String text, GptMarkdownConfig config) {
    final match = exp.firstMatch(text);
    final taskIndex = _seen++;
    final checked = (match?.group(1) ?? ' ').toLowerCase() == 'x';
    final content = match?.group(2)?.trim() ?? '';
    final onToggle = this.onToggle;

    return TaskCheckboxRow(
      checked: checked,
      onChanged: onToggle == null
          ? null
          : (value) => onToggle(taskIndex, value),
      child: MdWidget(context, content, true, config: config),
    );
  }
}

/// A checkbox and its task text on one line.
///
/// Stateless on purpose: the message body is the single source of truth, so
/// the box reflects the marker in the text and moves only once the edit that
/// rewrote it comes back. Holding a local optimistic value here would let the
/// UI disagree with the message it is rendering.
class TaskCheckboxRow extends StatelessWidget {
  const TaskCheckboxRow({
    super.key,
    required this.checked,
    required this.child,
    this.onChanged,
  });

  final bool checked;
  final Widget child;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final onChanged = this.onChanged;
    final interactive = onChanged != null;

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        // A bare Checkbox is a small tap target on a phone; the padding gives
        // it a comfortable one without pushing the text off the baseline.
        Padding(
          padding: const EdgeInsets.only(right: Grid.half, top: Grid.quarter),
          child: SizedBox(
            width: Grid.gutter,
            height: Grid.gutter,
            child: Checkbox(
              value: checked,
              visualDensity: VisualDensity.compact,
              materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
              onChanged: interactive
                  ? (value) => onChanged(value ?? false)
                  : null,
            ),
          ),
        ),
        Flexible(child: child),
      ],
    );
  }
}

/// The block-component list with [GfmTaskListMd] spliced in at the one
/// position that behaves.
///
/// Order is load-bearing, and both neighbours matter:
///
/// * it must come **after** `CodeBlockMd`, or a `- [ ]` written inside a fence
///   is rendered as a live checkbox instead of as the code sample it is —
///   which would also desync the ordinals from `countTaskMarkers`;
/// * it must come **before** `UnOrderedList`, whose `- anything` pattern would
///   otherwise claim the line first and leave the marker as literal text.
List<MarkdownComponent> taskAwareComponents({TaskToggleCallback? onToggle}) {
  final components = MarkdownComponent.globalComponents;
  final index = components.indexWhere((c) => c is UnOrderedList);
  final at = index < 0 ? components.length : index;
  return [
    ...components.sublist(0, at),
    GfmTaskListMd(onToggle: onToggle),
    ...components.sublist(at),
  ];
}
