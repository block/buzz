part of '../compose_bar.dart';

Future<void> _sendTextOnlyDraft({
  required BuildContext context,
  required _MarkdownEditingController controller,
  required ObjectRef<int> draftRevision,
  required int submittedDraftRevision,
  required VoidCallback cleanupStoredDraft,
  required FocusNode focusNode,
  required VoidCallback clearComposer,
  required Future<void> Function() addMentionedNonMembers,
  required _ComposeDraftPayload payload,
  required _OutgoingMentions outgoing,
  required ComposeBarOnSend onSend,
  required ScaffoldMessengerState? messenger,
}) async {
  TextEditingValue? clearedDraftText;
  List<_ComposerMention>? clearedDraftMentions;
  var clearedDraftMentionsInvalid = false;
  int? clearedDraftRevision;

  void restoreClearedDraft() {
    final text = clearedDraftText;
    final mentions = clearedDraftMentions;
    if (!context.mounted ||
        text == null ||
        mentions == null ||
        clearedDraftRevision == null ||
        draftRevision.value != clearedDraftRevision) {
      return;
    }
    controller.replaceDraft(
      text,
      mentions: mentions,
      mentionsInvalid: clearedDraftMentionsInvalid,
    );
    focusNode.requestFocus();
  }

  try {
    await addMentionedNonMembers();
    // A draft that changed while adding ran is not the draft the user sent.
    // Keep it and send nothing.
    if (context.mounted && draftRevision.value != submittedDraftRevision) {
      messenger?.showFeedbackSnackBar(
        const SnackBar(content: Text('Not sent. Message changed.')),
        kind: ToastKind.error,
      );
      return;
    }
    void clearSubmittedDraft() {
      if (!context.mounted || draftRevision.value != submittedDraftRevision) {
        return;
      }
      clearedDraftText = controller.value;
      clearedDraftMentions = controller.bindings;
      clearedDraftMentionsInvalid = controller.mentionsInvalid;
      clearComposer();
      clearedDraftRevision = draftRevision.value;
    }

    await ComposerSendOrigin.uponLocalInsertion(
      clearSubmittedDraft,
      () => onSend(
        payload.content,
        outgoing.pubkeys,
        mediaTags: [...payload.mediaTags, ...outgoing.referenceTags],
      ),
    );
    // A send destination without an optimistic timeline clears on success.
    if (clearedDraftRevision == null) {
      clearSubmittedDraft();
      if (clearedDraftRevision == null) cleanupStoredDraft();
    }
  } on ActiveCommunityChangedError {
    restoreClearedDraft();
    _reportSendCancelledByCommunitySwitch(messenger);
  } catch (error) {
    // The caller runs unawaited, so surface publish failures and restore the
    // sent draft unless the user has already started a new one.
    restoreClearedDraft();
    messenger?.showFeedbackSnackBar(
      SnackBar(content: Text(_sendFailureMessage(error))),
      kind: ToastKind.error,
    );
  }
}

void _useComposeDraftLifecycle({
  required BuildContext context,
  required ObjectRef<_ComposerVoiceNote> voiceNote,
  required WidgetRef ref,
  required _MarkdownEditingController controller,
  required String draftKey,
  required String channelId,
  required String? threadHeadId,
  required String draftIdentity,
  required ObjectRef<int> draftRevision,
  required ObjectRef<String?> prefillText,
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
  // Retire the old listener before restoring another scope's text: replacement
  // effects can run before the previous effect's cleanup.
  final owner = useMemoized(Object.new, [draftKey, draftIdentity]);
  final currentOwner = useRef(owner)..value = owner;
  final lastDraftIdentity = useRef<String?>(null);
  useEffect(() {
    final identity = '$draftIdentity\u0000$draftKey';
    final shouldHydrate = lastDraftIdentity.value != identity;
    final identityChanged =
        lastDraftIdentity.value != null && lastDraftIdentity.value != identity;
    lastDraftIdentity.value = identity;
    final saved = ref.read(composeDraftsProvider.notifier).draftFor(draftKey);
    // Text and bindings restore from one persisted snapshot, together.
    final restored = controller.restoredMentions(saved);
    void hydrate(String text) => controller.replaceDraft(
      TextEditingValue(text: text),
      mentions: restored,
      mentionsInvalid: saved?.mentionsInvalid ?? false,
    );
    if (identityChanged) {
      prefillText.value = null;
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
      hydrate(saved?.text ?? '');
    } else if (saved != null && controller.text.isEmpty) {
      hydrate(saved.text);
    } else if (shouldHydrate && saved == null) {
      controller.replaceDraft(controller.value, mentions: const []);
    }

    var lastPersistedText = controller.text;
    var lastBindings = [for (final m in controller.bindings) m.toDraft()];
    var lastInvalid = controller.mentionsInvalid;
    void persistDraft() {
      if (!identical(currentOwner.value, owner)) return;
      final text = controller.text;
      // An untouched agent prefill is not a draft.
      if (text == prefillText.value) return;
      // The editor already carried the bindings across the edit, so text and
      // bindings are one snapshot here.
      final bindings = [for (final m in controller.bindings) m.toDraft()];
      final invalid = controller.mentionsInvalid;
      if (text == lastPersistedText &&
          invalid == lastInvalid &&
          listEquals(bindings, lastBindings)) {
        return;
      }
      lastBindings = bindings;
      lastInvalid = invalid;
      lastPersistedText = text;
      draftRevision.value += 1;
      ref
          .read(composeDraftsProvider.notifier)
          .save(
            key: draftKey,
            channelId: channelId,
            threadHeadId: threadHeadId,
            text: text,
            mentions: bindings,
            mentionsInvalid: invalid,
            transcriptionId: voiceNote.value.insertedTranscriptionId.value,
          );
    }

    controller.addListener(persistDraft);
    // Another writer, such as a failed quick send from Siri, can save this
    // draft while the composer is open; show its text rather than overwrite
    // it with the next keystroke. Own saves match lastPersistedText.
    final external = ref.listenManual(
      composeDraftsProvider.select(
        (drafts) => drafts.where((d) => d.key == draftKey).firstOrNull,
      ),
      (_, saved) {
        if (!identical(currentOwner.value, owner)) return;
        if (saved == null ||
            ((saved.text == lastPersistedText ||
                    saved.text == controller.text) &&
                saved.mentionsInvalid == lastInvalid &&
                listEquals(saved.mentions, lastBindings))) {
          return;
        }
        // A newer draft: an ordinary send that fails must not restore over it.
        // Text and the keys its mentions are bound to change together.
        draftRevision.value += 1;
        lastPersistedText = saved.text;
        lastBindings = List.of(saved.mentions);
        lastInvalid = saved.mentionsInvalid;
        controller.replaceDraft(
          TextEditingValue(text: saved.text),
          mentions: controller.restoredMentions(saved),
          mentionsInvalid: saved.mentionsInvalid,
        );
      },
    );
    return () {
      controller.removeListener(persistDraft);
      external.close();
    };
  }, [controller, draftKey, draftIdentity, onDraftIdentityChanged]);
  _useComposerShareDelivery(
    context: context,
    ref: ref,
    draftKey: draftKey,
    controller: controller,
    attachments: attachments,
    draftRevision: draftRevision,
    voiceNote: voiceNote,
    uploadError: uploadError,
  );
}

// Report aggregate work independently of rebuilds, including sub-frame failures.
void _useComposerBusy({
  required ValueNotifier<bool> isSending,
  required ValueNotifier<int> uploadingCount,
  required ValueNotifier<int> preparingAttachments,
  required bool isDraft,
  required ValueChanged<bool>? onChanged,
}) {
  final busyCallback = useRef(onChanged);
  busyCallback.value = onChanged;
  useEffect(() {
    var active = true;
    void reportCurrent() {
      if (!active) return;
      busyCallback.value?.call(
        isSending.value ||
            uploadingCount.value > 0 ||
            (isDraft && preparingAttachments.value > 0),
      );
    }

    void reportBusy() {
      // Scope changes can reset the composer from a build-time effect.
      // Ordinary operation completions still release the parent immediately.
      if (SchedulerBinding.instance.schedulerPhase ==
          SchedulerPhase.persistentCallbacks) {
        WidgetsBinding.instance.addPostFrameCallback((_) => reportCurrent());
      } else {
        reportCurrent();
      }
    }

    final sources = [isSending, uploadingCount, preparingAttachments];
    for (final source in sources) {
      source.addListener(reportBusy);
    }
    return () {
      active = false;
      for (final source in sources) {
        source.removeListener(reportBusy);
      }
      // Disposal can happen during a parent build. Release after that frame.
      final callback = busyCallback.value;
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => callback?.call(false),
      );
    };
  }, [isSending, uploadingCount, preparingAttachments, isDraft]);
}
