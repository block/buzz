import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/outbox/text_outbox_store.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/confirmation_dialog.dart';
import '../../shared/widgets/feedback_snackbar.dart';
import 'text_outbox_provider.dart';

/// Keeps recoverable sends at the conversation tail while history reconnects.
class TextOutboxRows extends ConsumerWidget {
  const TextOutboxRows({
    super.key,
    required this.scope,
    required this.channelId,
    required this.visibleEventIds,
    this.parentId,
  });

  final TextOutboxScope scope;
  final String channelId;
  final String? parentId;
  final Iterable<String> visibleEventIds;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(textOutboxMessagesProvider(scope));
    if (state.hasError) {
      return const Padding(
        padding: EdgeInsets.all(Grid.sm),
        child: Text('Couldn’t open saved sends.'),
      );
    }
    final saved = state.value;
    if (saved == null || saved.isEmpty) return const SizedBox.shrink();
    final shown = visibleEventIds.toSet();
    final rows = [
      for (final row in saved)
        if (row.draft.channelId == channelId &&
            row.draft.parentId == parentId &&
            !shown.contains(row.signedEvent?.id))
          row,
    ];
    if (rows.isEmpty) return const SizedBox.shrink();
    return ConstrainedBox(
      constraints: const BoxConstraints(maxHeight: 160),
      child: ListView.builder(
        shrinkWrap: true,
        padding: const EdgeInsets.symmetric(horizontal: Grid.md),
        itemCount: rows.length,
        itemBuilder: (context, index) {
          final row = rows[index];
          return _QueuedMessageRow(
            key: ValueKey(row.id),
            row: row,
            scope: scope,
          );
        },
      ),
    );
  }
}

class _QueuedMessageRow extends ConsumerWidget {
  const _QueuedMessageRow({super.key, required this.row, required this.scope});

  final TextOutboxMessage row;
  final TextOutboxScope scope;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    Future<void> recover(Future<void> Function(TextOutboxStore) action) async {
      try {
        if (ref.read(textOutboxScopeProvider) == scope) {
          await action(ref.read(textOutboxStoreProvider));
        }
      } catch (_) {
        if (!context.mounted) return;
        ScaffoldMessenger.of(context).showFeedbackSnackBar(
          const SnackBar(content: Text('Couldn’t update the saved send.')),
          kind: ToastKind.error,
        );
      }
    }

    final label = switch (row.status) {
      TextOutboxStatus.pending => 'Pending',
      TextOutboxStatus.failed => 'Failed',
      TextOutboxStatus.unconfirmed => 'Delivery unconfirmed',
    };
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.xs),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Text(row.draft.text, maxLines: 4, overflow: TextOverflow.ellipsis),
          Text(label, style: context.textTheme.labelSmall),
          if (row.status != TextOutboxStatus.pending)
            Wrap(
              alignment: WrapAlignment.end,
              children: [
                if (row.status == TextOutboxStatus.failed)
                  TextButton(
                    onPressed: () =>
                        recover((store) => store.retry(scope, row.id)),
                    child: const Text('Retry'),
                  ),
                TextButton(
                  onPressed: () =>
                      Clipboard.setData(ClipboardData(text: row.draft.text)),
                  child: const Text('Copy text'),
                ),
                TextButton(
                  onPressed: () async {
                    final confirmed = await showDestructiveConfirmation(
                      context: context,
                      title: 'Discard saved send?',
                      message: 'This removes the saved text from this device.',
                      confirmLabel: 'Discard',
                    );
                    if (context.mounted && confirmed) {
                      await recover((store) => store.discard(scope, row.id));
                    }
                  },
                  child: const Text('Discard'),
                ),
              ],
            ),
        ],
      ),
    );
  }
}
