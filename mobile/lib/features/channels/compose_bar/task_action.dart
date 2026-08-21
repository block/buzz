part of '../compose_bar.dart';

/// Opens the "New task" sheet for [composer]'s channel and thread scope.
///
/// Kept out of `compose_bar_widget.dart` on purpose: that file owns the
/// composer's entire hook and send pipeline and is already at the repo's
/// 1000-line ceiling, so a new action wires in as a single delegating line
/// there and lives here — the same shape `_showComposerEmojiPicker` uses.
///
/// The composer does not track a `parentEventId` (the message's destination is
/// the parent's business, via `onSend`), so the task's `source_ref` is derived
/// from the thread ids the composer *does* carry: the thread head it is
/// replying under, falling back to the thread root.
void _openComposerTaskSheet(
  BuildContext context,
  WidgetRef ref,
  ComposeBar composer,
) {
  unawaited(
    showCreateTaskSheet(
      context: context,
      ref: ref,
      channelId: composer.channelId,
      channelName: composer.channelName,
      sourceEventId: composer.threadHeadId ?? composer.rootId,
    ),
  );
}
