/// The task detail sheet (HW-005 read surface, HW-007 status write surface).
///
/// HW-004 made "this thread produced a task" visible as a chip, but left the
/// task write-once and invisible: `getTask()` had zero production callers and
/// `TaskEvent` was parsed but never rendered. This sheet is the first
/// task-VIEWING surface on mobile — chip tap → one `GET /api/tasks/{id}` →
/// render `TaskDetail` (task + full event history).
///
/// HW-007 made the STATUS — and only the status — writable. Tapping it opens
/// [showTaskStatusPicker]; a different selection issues exactly one
/// `PATCH /api/tasks/{id}` carrying only `status`, then RE-FETCHES so the new
/// status and the relay-appended `status_changed` row both render from relay
/// truth. There is no optimistic local mutation: a failed write can never leave
/// a phantom success on screen. Title, priority, assignee and the comment
/// composer remain deliberately out of scope.
///
/// AUTHORIZATION IS INHERITED, NEVER INVENTED. The relay's PATCH path is
/// channel-membership scoped with no per-user ownership check, so the control
/// is never hidden on a guessed ownership rule — doing so would misrepresent
/// the real invariant. The sheet issues one request per open (plus one per
/// explicit retry, plus one PATCH + one re-fetch per accepted transition);
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
import 'task_status_picker.dart';
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
    return _TaskDetailView(
      detail: detail,
      onChanged: () => retryTick.value++,
    );
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

class _TaskDetailView extends HookConsumerWidget {
  const _TaskDetailView({required this.detail, required this.onChanged});

  final TaskDetail detail;

  /// Invoked after an accepted transition to force the sheet's single re-fetch.
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
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
                // Bounded so a long status or a hostile relay error message
                // cannot push the row past its width. The header Row gives no
                // width constraint of its own, so an unbounded Column here
                // overflows on a 10k-char error (caught by the write-error
                // clamp test, which measured a 6434px overflow).
                Flexible(
                  child: _TaskStatusControl(
                    taskId: detail.task.id,
                    status: detail.task.status,
                    onChanged: onChanged,
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

/// The interactive status control and the PATCH lifecycle behind it.
///
/// Owns the entire write path so the picker stays a pure selection widget.
/// The sequence is deliberately pessimistic: PATCH, and only on a relay ACK
/// re-fetch via [onChanged]. Nothing is mutated locally, so a rejected write
/// leaves the last relay-known status on screen rather than a phantom success.
class _TaskStatusControl extends HookConsumerWidget {
  const _TaskStatusControl({
    required this.taskId,
    required this.status,
    required this.onChanged,
  });

  final String taskId;
  final TaskStatus status;
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final isSending = useState(false);
    final writeError = useState<String?>(null);

    Future<void> pick() async {
      // One in-flight transition at a time: a double-tap must not produce two
      // PATCHes and two competing re-fetches.
      if (isSending.value) return;
      final selected = await showTaskStatusPicker(
        context: context,
        current: status,
      );
      // Dismissed without choosing, OR chose the status it already has. The
      // relay rejects an empty patch with 400 "patch must change at least one
      // field", so the no-op is declined HERE rather than sent and failed.
      if (selected == null || selected == status) return;

      isSending.value = true;
      writeError.value = null;
      try {
        // Exactly one field crosses the wire. No actor, role or ownership
        // claim is ever sent: the relay authorizes on channel membership.
        await ref.read(tasksApiProvider).updateTask(taskId, status: selected);
        onChanged();
      } on Object catch (error) {
        // Relay-supplied message: untrusted, clamped, plain text. The status
        // shown stays the last relay value because nothing was mutated.
        writeError.value = clampTaskDetailBody(error.toString());
      } finally {
        if (context.mounted) isSending.value = false;
      }
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.end,
      mainAxisSize: MainAxisSize.min,
      children: [
        InkWell(
          key: const ValueKey('task-detail-status'),
          // Disabled only while a write is in flight — never on a guessed
          // ownership rule, which would misrepresent the relay's real gate.
          onTap: isSending.value ? null : pick,
          borderRadius: BorderRadius.circular(8),
          child: Padding(
            padding: const EdgeInsets.symmetric(
              vertical: Grid.half,
              horizontal: Grid.half,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  threadTaskStatusLabel(status),
                  key: const ValueKey('task-detail-status-label'),
                  style: context.textTheme.labelMedium?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
                const SizedBox(width: Grid.quarter),
                if (isSending.value)
                  const SizedBox(
                    key: ValueKey('task-detail-status-pending'),
                    width: 12,
                    height: 12,
                    child: BuzzLoadingIndicator(size: 12),
                  )
                else
                  Icon(
                    LucideIcons.chevronDown,
                    key: const ValueKey('task-detail-status-caret'),
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  ),
              ],
            ),
          ),
        ),
        if (writeError.value case final message?)
          Padding(
            padding: const EdgeInsets.only(top: Grid.quarter),
            child: Text(
              message,
              key: const ValueKey('task-detail-status-error'),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.end,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.error,
              ),
            ),
          ),
      ],
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
