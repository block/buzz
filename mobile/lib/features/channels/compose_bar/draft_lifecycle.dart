part of '../compose_bar.dart';

class _ComposeSendCancelled implements Exception {
  const _ComposeSendCancelled();
}

Future<void> _sendTextOnlyDraft({
  required BuildContext context,
  required _MarkdownEditingController controller,
  required ObjectRef<Map<String, MentionCandidate>> mentionMap,
  required ObjectRef<int> draftRevision,
  required int submittedDraftRevision,
  required bool Function() ownsSource,
  required FocusNode focusNode,
  required VoidCallback clearComposer,
  required Future<void> Function() addMentionedNonMembers,
  required _ComposeDraftPayload payload,
  required _OutgoingMentions outgoing,
  required ComposeBarOnSend onSend,
  required ScaffoldMessengerState? messenger,
}) async {
  TextEditingValue? clearedDraftText;
  Map<String, MentionCandidate>? clearedDraftMentions;
  int? clearedDraftRevision;

  void restoreClearedDraft() {
    if (!ownsSource() ||
        clearedDraftText == null ||
        clearedDraftMentions == null ||
        clearedDraftRevision == null ||
        draftRevision.value != clearedDraftRevision) {
      return;
    }
    controller.value = clearedDraftText;
    mentionMap.value
      ..clear()
      ..addAll(clearedDraftMentions);
    focusNode.requestFocus();
  }

  try {
    await addMentionedNonMembers();
    if (!ownsSource()) return;
    // Clear before optimistic insertion so the outgoing row and draft never
    // appear simultaneously during the send transition. If the user edited
    // while membership changes were pending, preserve that newer draft.
    if (context.mounted && draftRevision.value == submittedDraftRevision) {
      clearedDraftText = controller.value;
      clearedDraftMentions = Map<String, MentionCandidate>.of(mentionMap.value);
      clearComposer();
      clearedDraftRevision = draftRevision.value;
    }
    await onSend(
      payload.content,
      outgoing.pubkeys,
      mediaTags: [...payload.mediaTags, ...outgoing.referenceTags],
    );
  } on _ComposeSendCancelled {
    restoreClearedDraft();
  } on StateError {
    restoreClearedDraft();
    _reportSendCancelledByCommunitySwitch(messenger);
  } catch (error) {
    // The caller runs unawaited, so surface publish failures and restore the
    // sent draft unless the user has already started a new one.
    restoreClearedDraft();
    messenger?.showSnackBar(
      SnackBar(content: Text(_composeSendErrorMessage(error))),
    );
  }
}

void _useComposeDraftLifecycle({
  required WidgetRef ref,
  required _MarkdownEditingController controller,
  required String draftKey,
  required String channelId,
  required String? threadHeadId,
  required String draftIdentity,
  required ObjectRef<int> draftRevision,
  required ValueNotifier<List<_PendingAttachment>> attachments,
  required ObjectRef<int> uploadGeneration,
  required ObjectRef<UploadCancellationToken?> activeUploadCancellation,
  required ValueNotifier<int> uploadingCount,
  required ValueNotifier<bool> isSending,
  required ValueNotifier<_AttachmentSurface> attachmentSurface,
  required ValueNotifier<String?> uploadError,
  required _IOSAttachmentPopoverController iosAttachmentPopover,
  required VoidCallback onDraftIdentityChanged,
}) {
  // Hook disposal can lag the replacement effect. An old text listener must
  // not persist the incoming draft into its previous channel/account scope.
  final owner = useMemoized(Object.new, [draftKey, draftIdentity]);
  final currentOwner = useRef(owner)..value = owner;
  final lastDraftIdentity = useRef<String?>(null);
  final lastDraftKey = useRef<String?>(null);
  useEffect(() {
    final identityChanged =
        lastDraftIdentity.value != null &&
        (lastDraftIdentity.value != draftIdentity ||
            lastDraftKey.value != draftKey);
    lastDraftIdentity.value = draftIdentity;
    lastDraftKey.value = draftKey;
    final saved = ref.read(composeDraftsProvider.notifier).textFor(draftKey);
    if (identityChanged) {
      draftRevision.value += 1;
      onDraftIdentityChanged();
      uploadGeneration.value += 1;
      activeUploadCancellation.value?.cancel();
      activeUploadCancellation.value = null;
      uploadingCount.value = 0;
      isSending.value = false;
      attachmentSurface.value = _AttachmentSurface.closed;
      uploadError.value = null;
      unawaited(iosAttachmentPopover.dispose());
      final staleAttachments = attachments.value;
      attachments.value = const [];
      unawaited(_deleteOwnedAttachments(staleAttachments));
      controller.text = saved ?? '';
    } else if (saved != null && controller.text.isEmpty) {
      controller.text = saved;
    }

    var lastPersistedText = controller.text;
    void persistDraft() {
      if (!identical(currentOwner.value, owner)) return;
      final text = controller.text;
      if (text == lastPersistedText) return;
      lastPersistedText = text;
      draftRevision.value += 1;
      ref
          .read(composeDraftsProvider.notifier)
          .save(
            key: draftKey,
            channelId: channelId,
            threadHeadId: threadHeadId,
            text: text,
          );
    }

    controller.addListener(persistDraft);
    return () => controller.removeListener(persistDraft);
  }, [controller, draftKey, draftIdentity, onDraftIdentityChanged]);
}
