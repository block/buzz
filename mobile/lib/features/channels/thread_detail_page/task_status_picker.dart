/// The status picker behind the task detail sheet's write surface (HW-007).
///
/// HW-005 shipped the read-only detail sheet and left a marker naming this
/// follow-up: the status was rendered as static text, so a task produced from a
/// mobile thread was permanently `todo` from the phone. This picker is the
/// selection half of that write surface — it CHOOSES a status and returns it.
/// It performs no network call itself, which is what keeps it unit-testable in
/// isolation and keeps the sheet the single owner of the PATCH lifecycle.
///
/// AUTHORIZATION IS DELIBERATELY NOT MODELLED HERE. The relay's
/// `PATCH /api/tasks/{id}` is channel-membership scoped with no per-user
/// ownership check (`authorize_task_request` → `enforce_channel_access` against
/// the task's CURRENT channel). Every status is therefore offered to anyone who
/// can see the task. A client-side ownership gate would misrepresent the real
/// invariant by hiding an option the relay would have accepted.
library;

import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../../shared/tasks/task.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/modal_presentation.dart';
import 'thread_task_chip.dart';

/// Opens the status picker and resolves to the chosen status.
///
/// Resolves to `null` when the sheet is dismissed without a choice. The CURRENT
/// status is still selectable and still returned: swallowing it here would hide
/// the no-op decision from the caller, so the caller — which owns the request —
/// is the one that declines to send it.
Future<TaskStatus?> showTaskStatusPicker({
  required BuildContext context,
  required TaskStatus current,
}) {
  return showBuzzModalBottomSheet<TaskStatus>(
    context: context,
    title: 'Task status',
    showDragHandle: true,
    constraints: const BoxConstraints(maxWidth: 640),
    builder: (sheetContext) => TaskStatusPicker(
      current: current,
      onSelected: (status) => Navigator.of(sheetContext).pop(status),
    ),
  );
}

/// Lists every [TaskStatus] with the current one indicated.
///
/// Split out as a public widget so the list, the selection marker and the
/// callback contract can be tested without a relay, a sheet route, or a pump of
/// the whole detail surface.
class TaskStatusPicker extends StatelessWidget {
  /// Creates a picker showing [current] as selected.
  const TaskStatusPicker({
    required this.current,
    required this.onSelected,
    super.key,
  });

  /// The task's status as last read from the relay.
  final TaskStatus current;

  /// Called with the tapped status, including when it equals [current].
  final ValueChanged<TaskStatus> onSelected;

  @override
  Widget build(BuildContext context) {
    return Padding(
      key: const ValueKey('task-status-picker'),
      padding: const EdgeInsets.fromLTRB(Grid.gutter, 0, Grid.gutter, Grid.xs),
      child: SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Every status is offered: the relay gates on channel membership,
            // never on ownership, so there is nothing to hide per-user.
            for (final status in TaskStatus.values)
              _TaskStatusOption(
                status: status,
                isCurrent: status == current,
                onSelected: onSelected,
              ),
          ],
        ),
      ),
    );
  }
}

class _TaskStatusOption extends StatelessWidget {
  const _TaskStatusOption({
    required this.status,
    required this.isCurrent,
    required this.onSelected,
  });

  final TaskStatus status;
  final bool isCurrent;
  final ValueChanged<TaskStatus> onSelected;

  @override
  Widget build(BuildContext context) {
    final label = threadTaskStatusLabel(status);
    return InkWell(
      key: ValueKey('task-status-option-${status.wireValue}'),
      onTap: () => onSelected(status),
      borderRadius: BorderRadius.circular(10),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          vertical: Grid.twelve,
          horizontal: Grid.xxs,
        ),
        child: Row(
          children: [
            Expanded(
              child: Text(
                label,
                style: context.textTheme.bodyLarge?.copyWith(
                  fontWeight: isCurrent ? FontWeight.w600 : FontWeight.w400,
                ),
              ),
            ),
            // The marker is an icon AND a semantics flag, so the current status
            // is not conveyed by weight alone.
            if (isCurrent)
              Icon(
                LucideIcons.check,
                key: ValueKey('task-status-current-${status.wireValue}'),
                size: 18,
                color: context.colors.primary,
                semanticLabel: 'Current status',
              ),
          ],
        ),
      ),
    );
  }
}
