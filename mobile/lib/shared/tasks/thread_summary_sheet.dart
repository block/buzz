/// The "Summarize thread" bottom sheet.
///
/// Shows the digest [summarizeThread] produced, and lets the reader persist it
/// against a task. The relay stores at most one `summary_persisted` event per
/// task (`idx_task_events_one_summary_per_task`), so a second attempt on the
/// same task comes back as a 400 and is surfaced verbatim rather than retried.
library;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../clipboard_utils.dart';
import '../theme/theme.dart';
import '../widgets/app_list.dart';
import '../widgets/app_list_card.dart';
import '../widgets/buzz_loading_indicator.dart';
import '../widgets/modal_presentation.dart';
import '../widgets/sheet_divider.dart';
import 'task.dart';
import 'thread_summary.dart';
import 'tasks_api.dart';

/// Opens the summary sheet for an already-collected thread.
///
/// [messages] is the transcript in thread order; the caller owns collecting it,
/// because only the caller knows which view's messages are on screen.
Future<void> showThreadSummarySheet({
  required BuildContext context,
  required WidgetRef ref,
  required String channelId,
  required List<ThreadMessageDigest> messages,
  String? sourceRef,
}) {
  return showBuzzModalBottomSheet<void>(
    context: context,
    title: 'Thread summary',
    isScrollControlled: true,
    showDragHandle: true,
    constraints: BoxConstraints(
      maxWidth: 640,
      maxHeight: MediaQuery.sizeOf(context).height * 0.9,
    ),
    builder: (_) =>
        _ThreadSummarySheet(
          channelId: channelId,
          messages: messages,
          sourceRef: sourceRef,
        ),
  );
}

class _ThreadSummarySheet extends HookConsumerWidget {
  const _ThreadSummarySheet({
    required this.channelId,
    required this.messages,
    this.sourceRef,
  });

  final String channelId;
  final List<ThreadMessageDigest> messages;

  /// The thread key this sheet was opened from, written as a task's
  /// `source_ref` by the composer as `threadHeadId ?? rootId`.
  final String? sourceRef;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Pure and deterministic, so it is memoized on the transcript rather than
    // recomputed on every rebuild of the sheet.
    final summary = useMemoized(() => summarizeThread(messages), [messages]);
    final isSaving = useState(false);
    final actionError = useState<String?>(null);
    final savedTo = useState<String?>(null);

    Future<void> save() async {
      final messenger = ScaffoldMessenger.of(context);
      final api = ref.read(tasksApiProvider);

      // The thread this sheet was opened from may already have produced a
      // task. `source_ref` records exactly that, written by the composer as
      // `threadHeadId ?? rootId`, so querying by the same expression matches
      // by construction. A unique hit is the answer and the picker — whose
      // 20-row recency window can simply omit the right task in a busy
      // channel — is skipped entirely.
      SummaryTaskTarget? target;
      if (sourceRef case final key?) {
        try {
          final linked = await api.listTasks(channelId: channelId, sourceRef: key);
          if (linked.length == 1) target = SummaryTaskTarget(linked.single);
        } on Exception {
          // A failed reverse lookup is not a failed save: fall through to the
          // picker rather than blocking on an optional convenience.
          target = null;
        }
      }
      if (target == null) {
        if (!context.mounted) return;
        target = await showSummaryTaskPicker(
          context: context,
          ref: ref,
          channelId: channelId,
        );
      }
      if (target == null || !context.mounted) return;

      isSaving.value = true;
      actionError.value = null;
      try {
        final task =
            target.task ??
            await api.createTask(
              title: threadTaskTitle(messages),
              channelId: channelId,
              sourceRef: sourceRef,
            );
        await api.appendTaskEvent(
          task.id,
          action: TaskEventAction.summaryPersisted,
          body: summary,
        );
        messenger.showSnackBar(
          const SnackBar(content: Text('Summary saved to task')),
        );
        // Guarded because this sheet stays dismissible while the two writes
        // are in flight; the snackbar above goes through the messenger
        // resolved before them, so it lands either way.
        if (context.mounted) savedTo.value = task.title;
      } catch (error) {
        if (context.mounted) actionError.value = error.toString();
      } finally {
        if (context.mounted) isSaving.value = false;
      }
    }

    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.gutter, 0, Grid.gutter, Grid.xs),
      child: SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Flexible(
              child: SingleChildScrollView(
                key: const ValueKey('thread-summary-body'),
                child: GptMarkdown(
                  summary,
                  style: context.textTheme.bodyMedium,
                ),
              ),
            ),
            if (savedTo.value case final title?)
              Padding(
                padding: const EdgeInsets.only(top: Grid.xxs),
                child: Text(
                  'Saved to “$title”.',
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ),
            if (actionError.value case final error?)
              Padding(
                padding: const EdgeInsets.only(top: Grid.xxs),
                child: Text(
                  error,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ),
            const SheetDivider(),
            Row(
              children: [
                Expanded(
                  child: OutlinedButton.icon(
                    key: const ValueKey('thread-summary-copy'),
                    onPressed: () => copyToClipboard(
                      context,
                      summary,
                      message: 'Summary copied to clipboard',
                    ),
                    icon: const Icon(LucideIcons.copy, size: 18),
                    label: const Text('Copy'),
                  ),
                ),
                const SizedBox(width: Grid.half),
                Expanded(
                  child: FilledButton.icon(
                    key: const ValueKey('thread-summary-save'),
                    // Re-saving to the same task cannot succeed, so the action
                    // retires once this sheet has persisted a summary.
                    onPressed: isSaving.value || savedTo.value != null
                        ? null
                        : save,
                    icon: isSaving.value
                        ? const BuzzLoadingIndicator(
                            size: 16,
                            semanticLabel: 'Saving summary',
                          )
                        : const Icon(LucideIcons.listTodo, size: 18),
                    label: Text(isSaving.value ? 'Saving…' : 'Save to task'),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// Where a summary should be persisted: an existing [task], or a new one.
@immutable
class SummaryTaskTarget {
  /// Names an existing task, or a new one when [task] is null.
  const SummaryTaskTarget(this.task);

  /// The chosen task, or null to open a fresh one for this thread.
  final Task? task;
}

/// Asks which task a summary belongs to.
///
/// Lists this channel's tasks and offers a new one. Whether a listed task
/// already holds a summary is not shown, because knowing would cost one
/// `GET /api/tasks/{id}` per row; the relay's 400 on the second write is the
/// authority, and it is reported as-is.
Future<SummaryTaskTarget?> showSummaryTaskPicker({
  required BuildContext context,
  required WidgetRef ref,
  required String channelId,
}) {
  return showBuzzModalBottomSheet<SummaryTaskTarget>(
    context: context,
    title: 'Save summary to',
    isScrollControlled: true,
    showDragHandle: true,
    constraints: BoxConstraints(
      maxWidth: 640,
      maxHeight: MediaQuery.sizeOf(context).height * 0.7,
    ),
    builder: (_) => _SummaryTaskPicker(channelId: channelId),
  );
}

class _SummaryTaskPicker extends HookConsumerWidget {
  const _SummaryTaskPicker({required this.channelId});

  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final api = ref.read(tasksApiProvider);
    final tasks = useMemoized(
      () => api.listTasks(channelId: channelId, limit: 20),
      [channelId],
    );
    final snapshot = useFuture(tasks);

    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.gutter, 0, Grid.gutter, Grid.xs),
      child: SafeArea(
        top: false,
        child: ListView(
          shrinkWrap: true,
          children: [
            AppListCard(
              children: [
                AppListRow(
                  key: const ValueKey('summary-target-new-task'),
                  icon: LucideIcons.plus,
                  title: 'New task from this thread',
                  onTap: () =>
                      Navigator.of(context).pop(const SummaryTaskTarget(null)),
                ),
              ],
            ),
            if (snapshot.connectionState == ConnectionState.waiting)
              const Padding(
                padding: EdgeInsets.all(Grid.xs),
                child: Center(
                  child: BuzzLoadingIndicator(
                    size: 32,
                    semanticLabel: 'Loading tasks',
                  ),
                ),
              )
            else if (snapshot.error != null)
              Padding(
                padding: const EdgeInsets.all(Grid.xs),
                child: Text(
                  '${snapshot.error}',
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              )
            else if (snapshot.data case final loaded? when loaded.isNotEmpty)
              AppListCard(
                label: 'Existing tasks',
                children: [
                  for (final task in loaded)
                    AppListRow(
                      key: ValueKey('summary-target-${task.id}'),
                      icon: LucideIcons.listTodo,
                      title: task.title,
                      subtitle: task.status.wireValue,
                      onTap: () =>
                          Navigator.of(context).pop(SummaryTaskTarget(task)),
                    ),
                ],
              ),
          ],
        ),
      ),
    );
  }
}
