part of '../compose_bar.dart';

/// Adds a share waiting in [composerShareInboxProvider] for this composer's
/// draft. When the draft already has content, the person chooses whether the
/// share joins it or replaces it.
void _useComposerShareDelivery({
  required BuildContext context,
  required WidgetRef ref,
  required String draftKey,
  required _MarkdownEditingController controller,
  required ValueNotifier<List<_PendingAttachment>> attachments,
  required ObjectRef<int> draftRevision,
  required ObjectRef<_ComposerVoiceNote> voiceNote,
  required ValueNotifier<String?> uploadError,
}) {
  final waiting = ref.watch(
    composerShareInboxProvider.select((shares) => shares[draftKey]?.first.id),
  );
  final delivering = useRef(false);
  useEffect(() {
    if (waiting == null || delivering.value) return null;
    // Runs after the draft lifecycle restores this composer's saved text.
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (!context.mounted || delivering.value) return;
      // One share at a time, so each gets its own Add or Replace answer.
      delivering.value = true;
      try {
        while (context.mounted) {
          final share = ref
              .read(composerShareInboxProvider.notifier)
              .take(draftKey);
          if (share == null) break;
          await _deliverShare(
            context,
            share,
            controller,
            attachments,
            draftRevision,
            voiceNote,
            uploadError,
          );
        }
      } finally {
        delivering.value = false;
      }
    });
    return null;
  }, [waiting, draftKey]);
}

Future<void> _deliverShare(
  BuildContext context,
  PendingShare share,
  _MarkdownEditingController controller,
  ValueNotifier<List<_PendingAttachment>> attachments,
  ObjectRef<int> draftRevision,
  ObjectRef<_ComposerVoiceNote> voiceNote,
  ValueNotifier<String?> uploadError,
) async {
  var files = const <XFile>[];
  // Items the extension already failed to load count as failed too.
  var filesFailed = share.missed > 0;
  try {
    // The composer owns its copies; the app group's are deleted right away.
    // The pending share is only a hand-off: the items stay in the app they
    // were shared from, so a failed copy is re-shared rather than retried.
    files = await retainTemporaryImages([
      for (final path in share.files) XFile(path),
    ]);
  } catch (error) {
    debugPrint('share: could not copy shared files: $error');
    filesFailed = true;
  } finally {
    unawaited(deletePendingShare(share.id));
  }
  final text = share.composerText;
  if (text.isEmpty && files.isEmpty) {
    // Nothing survived, so the draft stays as it was.
    if (context.mounted && filesFailed) {
      uploadError.value = 'Couldn’t add some of what you shared.';
    }
    return;
  }
  final hasDraft =
      controller.text.trim().isNotEmpty || attachments.value.isNotEmpty;
  final replace =
      context.mounted && hasDraft && await _confirmReplaceDraft(context);
  if (!context.mounted) {
    for (final file in files) {
      await _deleteXFile(file);
    }
    return;
  }
  if (replace) {
    draftRevision.value += 1;
    final removed = attachments.value;
    attachments.value = const [];
    unawaited(_deleteOwnedAttachments(removed));
    controller.clear();
  }
  if (text.isNotEmpty) {
    final existing = controller.text.trimRight();
    controller.value = TextEditingValue(
      text: existing.isEmpty ? text : '$existing\n$text',
      selection: TextSelection.collapsed(
        offset: existing.isEmpty
            ? text.length
            : existing.length + 1 + text.length,
      ),
    );
  }
  if (files.isNotEmpty &&
      !_queueComposerImages(
        files,
        voiceNote.value,
        attachments,
        uploadError,
        draftRevision,
        true,
      )) {
    for (final file in files) {
      await _deleteXFile(file);
    }
    return;
  }
  // After queueing, which clears any earlier upload error.
  if (filesFailed) uploadError.value = 'Couldn’t add some of what you shared.';
}

/// Asks whether a share replaces the draft already in this conversation.
/// Dismissing keeps the draft and adds to it.
Future<bool> _confirmReplaceDraft(BuildContext context) async =>
    await showBuzzDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog.adaptive(
        title: const Text('Add to your draft?'),
        content: const Text(
          'This conversation already has a draft. Add what you shared to it, '
          'or replace it.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext, true),
            child: const Text('Replace'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(dialogContext, false),
            child: const Text('Add'),
          ),
        ],
      ),
    ) ??
    false;
