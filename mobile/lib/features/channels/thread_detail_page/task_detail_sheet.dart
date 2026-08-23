/// The read-only task detail sheet (HW-005).
///
/// HW-004 made "this thread produced a task" visible as a chip, but left the
/// task write-once and invisible: `getTask()` had zero production callers and
/// `TaskEvent` was parsed but never rendered. This sheet is the first
/// task-VIEWING surface on mobile — chip tap → one `GET /api/tasks/{id}` →
/// render `TaskDetail` (task + full event history).
///
/// READ-ONLY BY DESIGN. No PATCH, no POST, no status transition, no comment
/// composer — the write surface is HW-007 and deliberately out of scope here.
/// The sheet issues exactly one request per open (plus one per explicit retry);
/// there is no polling and no live subscription.
///
/// Untrusted-text rules match HW-003/HW-004: title, bodies and actors are
/// relay-supplied, length-clamped by runes (never UTF-16 code units), and
/// rendered as plain text — never markdown, never a link.
library;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../../shared/tasks/task.dart';
import '../../../shared/tasks/tasks_api.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/utils/string_utils.dart';
import '../../../shared/widgets/buzz_loading_indicator.dart';
import '../../../shared/widgets/modal_presentation.dart';
import '../../../shared/widgets/sheet_divider.dart';
import 'thread_task_chip.dart';

/// Longest task-event body rendered inside the sheet.
///
/// The relay caps a comment at the same order of magnitude, but the sheet is
/// the last line of defence for untrusted text: clamp here so a hostile or
/// future-relay body cannot grow the sheet without bound. Counted in runes for
/// the same reason [threadTaskChipTitleChars] is.
const taskDetailBodyChars = 500;

/// Clamps an untrusted task-event body to [taskDetailBodyChars] runes.
String clampTaskDetailBody(String body) {
  final trimmed = body.trim();
  final runes = trimmed.runes.toList();
  if (runes.length <= taskDetailBodyChars) return trimmed;
  return String.fromCharCodes(runes.take(taskDetailBodyChars));
}

/// Human label for a wire status string, or the raw string when unknown.
///
/// [TaskStatus.fromWire] degrades unknown values to `todo` so list rendering
/// survives rolling upgrades — but THIS surface states "from → to" transitions,
/// where a silent substitution would fabricate history that never happened.
/// Unknown statuses therefore pass through as plain text, never guessed.
String? taskWireStatusLabel(String? wire) {
  if (wire == null) return null;
  for (final status in TaskStatus.values) {
    if (status.wireValue == wire) return threadTaskStatusLabel(status);
  }
  return wire;
}

/// Opens the read-only detail sheet for one task.
Future<void> showTaskDetailSheet({
  required BuildContext context,
  required WidgetRef ref,
  required String taskId,
}) {
  return showBuzzModalBottomSheet<void>(
    context: context,
    title: 'Task detail',
    isScrollControlled: true,
    showDragHandle: true,
    constraints: BoxConstraints(
      maxWidth: 640,
      maxHeight: MediaQuery.sizeOf(context).height * 0.9,
    ),
    builder: (_) => _TaskDetailSheet(taskId: taskId),
  );
}

class _TaskDetailSheet extends HookConsumerWidget {
  const _TaskDetailSheet({required this.taskId});

  final String taskId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // One fetch per open, plus one per explicit retry tap. A tick, not a
    // timer: no polling, no live subscription, no background refresh.
    final retryTick = useState(0);
    final detailSnapshot = useFuture(
      useMemoized(
        () => ref.read(tasksApiProvider).getTask(taskId),
        [taskId, retryTick.value],
      ),
    );

    if (detailSnapshot.hasError) {
      return _TaskDetailError(
        error: detailSnapshot.error!,
        onRetry: () => retryTick.value++,
      );
    }
    final detail = detailSnapshot.data;
    if (detail == null) {
      return const Padding(
        padding: EdgeInsets.symmetric(vertical: Grid.xl),
        child: Center(
          key: ValueKey('task-detail-loading'),
          child: BuzzLoadingIndicator(size: 32),
        ),
      );
    }
    return _TaskDetailView(detail: detail);
  }
}

/// Failure state: an honest error row with an explicit retry.
///
/// A failed fetch (offline / 500) and an inaccessible task (404 from the
/// relay's channel-access gate) render IDENTICALLY here, so the sheet is not
/// an existence oracle, and a failure is never misrendered as "no task".
class _TaskDetailError extends StatelessWidget {
  const _TaskDetailError({required this.error, required this.onRetry});

  final Object error;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final message = error.toString();
    return Padding(
      key: const ValueKey('task-detail-error'),
      padding: const EdgeInsets.fromLTRB(Grid.gutter, Grid.xs, Grid.gutter, 0),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            LucideIcons.circleAlert,
            size: 20,
            color: context.colors.onSurfaceVariant,
          ),
          const SizedBox(height: Grid.half),
          const Text("Couldn't load this task."),
          if (message.trim().isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: Grid.quarter),
              child: Text(
                // Relay-supplied message: untrusted, clamped, plain text.
                clampTaskDetailBody(message),
                key: const ValueKey('task-detail-error-message'),
                maxLines: 3,
                overflow: TextOverflow.ellipsis,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          TextButton.icon(
            key: const ValueKey('task-detail-retry'),
            onPressed: onRetry,
            icon: const Icon(LucideIcons.refreshCcw, size: 16),
            label: const Text('Retry'),
          ),
        ],
      ),
    );
  }
}

class _TaskDetailView extends StatelessWidget {
  const _TaskDetailView({required this.detail});

  final TaskDetail detail;

  @override
  Widget build(BuildContext context) {
    final summaryBody = detail.summary?.body?.trim();
    // The summary event is promoted to its own card above; rendering it again
    // as a history row would show the same text twice. Every OTHER event
    // renders exactly one row.
    final history = [
      for (final event in detail.events)
        if (!event.isSummary) event,
    ];

    return Padding(
      key: const ValueKey('task-detail-content'),
      padding: const EdgeInsets.fromLTRB(Grid.gutter, 0, Grid.gutter, Grid.xs),
      child: SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                Icon(
                  LucideIcons.circleCheckBig,
                  size: 18,
                  color: context.colors.primary,
                ),
                const SizedBox(width: Grid.xxs),
                Expanded(
                  child: Text(
                    // Plain text on purpose: never markdown, never a link.
                    clampThreadTaskTitle(detail.task.title),
                    key: const ValueKey('task-detail-title'),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
                const SizedBox(width: Grid.xxs),
                Text(
                  threadTaskStatusLabel(detail.task.status),
                  key: const ValueKey('task-detail-status'),
                  style: context.textTheme.labelMedium?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
            ),
            if (summaryBody != null && summaryBody.isNotEmpty) ...[
              const SizedBox(height: Grid.xxs),
              DecoratedBox(
                key: const ValueKey('task-detail-summary'),
                decoration: BoxDecoration(
                  color: context.colors.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(10),
                ),
                child: Padding(
                  padding: const EdgeInsets.all(Grid.xxs + Grid.half),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        'Summary',
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                      const SizedBox(height: Grid.quarter),
                      Text(
                        clampTaskDetailBody(summaryBody),
                        key: const ValueKey('task-detail-summary-body'),
                        style: context.textTheme.bodyMedium,
                      ),
                    ],
                  ),
                ),
              ),
            ],
            // Summary absent → the section is simply omitted: no placeholder,
            // no "no summary" claim, no layout hole.
            if (history.isNotEmpty) ...[
              const SheetDivider(),
              Flexible(
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      for (final event in history) _TaskEventRow(event: event),
                    ],
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// One compact row per lifecycle event.
///
/// `TaskEvent.action` is deliberately unconstrained TEXT (`task_events.action`
/// is free TEXT so a new harness can ship without a migration), so unknown
/// actions render as a neutral row — never an error, never a crash.
class _TaskEventRow extends StatelessWidget {
  const _TaskEventRow({required this.event});

  final TaskEvent event;

  IconData get _icon => switch (event.action) {
    'created' => LucideIcons.circleDot,
    'status_changed' => LucideIcons.arrowLeftRight,
    'assigned' => LucideIcons.userPlus,
    'commented' => LucideIcons.messageSquareText,
    'title_changed' => LucideIcons.pencil,
    _ => LucideIcons.ellipsis,
  };

  String get _text => switch (event.action) {
    'created' => 'Created',
    'status_changed' =>
      // Labels when known, raw wire strings when not — never a guess that
      // would fabricate a transition this build has never heard of.
      '${taskWireStatusLabel(event.fromStatus) ?? '—'} → '
          '${taskWireStatusLabel(event.toStatus) ?? '—'}',
    'assigned' => 'Assigned',
    'title_changed' => _titleChangeText(),
    'commented' => _commentText(),
    _ => event.action,
  };

  String _titleChangeText() {
    final newTitle = event.body?.trim();
    if (newTitle == null || newTitle.isEmpty) return 'Title changed';
    return 'Title changed to “${clampThreadTaskTitle(newTitle)}”';
  }

  String _commentText() {
    final body = event.body?.trim();
    if (body == null || body.isEmpty) return 'Commented';
    return clampTaskDetailBody(body);
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      key: ValueKey('task-event-row-${event.id}'),
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Icon(
              _icon,
              size: 16,
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  // Plain text on purpose: bodies and actions are untrusted.
                  _text,
                  maxLines: 4,
                  overflow: TextOverflow.ellipsis,
                  style: context.textTheme.bodyMedium,
                ),
                if (event.actor case final actor? when actor.isNotEmpty)
                  Text(
                    // Truncated hex pubkey, plain text — never a profile link.
                    shortPubkey(actor),
                    key: ValueKey('task-event-actor-${event.id}'),
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
