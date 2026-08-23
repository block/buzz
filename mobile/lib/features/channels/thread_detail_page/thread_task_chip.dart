import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../../shared/tasks/task.dart';
import '../../../shared/theme/theme.dart';

/// Longest task title rendered inside the chip.
///
/// Titles are relay-supplied and therefore untrusted. The relay already caps
/// them at [maxTaskTitleChars], but the chip is a one-line signal sitting above
/// the transcript, so it clamps far shorter and lets [Text] ellipsize rather
/// than growing the thread header. Counted in runes for the same reason
/// `taskTitleLength` is: `String.length` counts UTF-16 code units and would
/// split an emoji or an astral-plane character in half.
const threadTaskChipTitleChars = 80;

/// Human label for a task's lifecycle state.
String threadTaskStatusLabel(TaskStatus status) => switch (status) {
  TaskStatus.todo => 'To do',
  TaskStatus.inProgress => 'In progress',
  TaskStatus.blocked => 'Blocked',
  TaskStatus.done => 'Done',
  TaskStatus.cancelled => 'Cancelled',
};

/// Clamps an untrusted task title to [threadTaskChipTitleChars] runes.
String clampThreadTaskTitle(String title) {
  final trimmed = title.trim();
  final runes = trimmed.runes.toList();
  if (runes.length <= threadTaskChipTitleChars) return trimmed;
  return String.fromCharCodes(runes.take(threadTaskChipTitleChars));
}

/// Read-only banner saying "this thread already produced a task".
///
/// The widget itself stays NON-INTERACTIVE by design: there is still no
/// `onTap`, no [InkWell], no [GestureDetector] and no navigation anywhere in
/// this subtree, and `thread_task_chip_test.dart` asserts that absence — a
/// display widget that owns no gestures keeps its standalone test meaningful.
///
/// The TAP AFFORDANCE lives at the page instead (HW-005):
/// `thread_detail_page.dart` wraps this chip in an `InkWell` that opens the
/// read-only task detail sheet (`task_detail_sheet.dart`), so opening the task
/// is tested where the gesture actually lives. This widget's contract — render
/// the fact, claim nothing about navigation — is unchanged.
///
/// Renders nothing when there is no task: callers pass a non-null [task] only
/// when the reverse lookup actually matched, so an empty result produces no
/// placeholder, no empty state and no layout shift.
class ThreadTaskChip extends StatelessWidget {
  /// The task this thread produced — the most recently updated one when the
  /// lookup matched several.
  final Task task;

  /// How many FURTHER tasks share this thread's `source_ref`, beyond [task].
  ///
  /// Rendered as an inert "+N" suffix. It is a count, not a control: there is
  /// no picker and no expansion, consistent with the non-interactive rule.
  final int additionalCount;

  const ThreadTaskChip({
    super.key,
    required this.task,
    this.additionalCount = 0,
  });

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 0),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: colors.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(10),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                LucideIcons.circleCheckBig,
                size: 15,
                color: colors.primary,
              ),
              const SizedBox(width: 8),
              Flexible(
                child: Text(
                  // Plain text on purpose: never markdown, never a link.
                  clampThreadTaskTitle(task.title),
                  key: const ValueKey('thread-task-chip-title'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: colors.onSurface,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
              const SizedBox(width: 8),
              Text(
                threadTaskStatusLabel(task.status),
                key: const ValueKey('thread-task-chip-status'),
                style: Theme.of(context).textTheme.bodySmall?.copyWith(
                  color: colors.onSurfaceVariant,
                ),
              ),
              if (additionalCount > 0) ...[
                const SizedBox(width: 6),
                Text(
                  '+$additionalCount',
                  key: const ValueKey('thread-task-chip-more'),
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: colors.onSurfaceVariant,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
