part of '../compose_bar.dart';

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
}) {
  final lastDraftIdentity = useRef<String?>(null);
  useEffect(() {
    final identityChanged =
        lastDraftIdentity.value != null &&
        lastDraftIdentity.value != draftIdentity;
    lastDraftIdentity.value = draftIdentity;
    final saved = ref.read(composeDraftsProvider.notifier).textFor(draftKey);
    if (identityChanged) {
      draftRevision.value += 1;
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

    void persistDraft() {
      draftRevision.value += 1;
      ref
          .read(composeDraftsProvider.notifier)
          .save(
            key: draftKey,
            channelId: channelId,
            threadHeadId: threadHeadId,
            text: controller.text,
          );
    }

    controller.addListener(persistDraft);
    return () => controller.removeListener(persistDraft);
  }, [controller, draftKey, draftIdentity]);
}
