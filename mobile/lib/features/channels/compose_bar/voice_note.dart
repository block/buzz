part of '../compose_bar.dart';

class _ComposerVoiceNote {
  const _ComposerVoiceNote({
    required this.start,
    required this.startTranscription,
    required this.transcriptionPanel,
    required this.isDictating,
    required this.transcription,
    required this.onKeyboardHidden,
    required this.onDraftIdentityChanged,
    required this.insertedTranscriptionId,
    required ValueNotifier<bool> isPreparing,
    required ValueNotifier<bool> isRecording,
    required ValueChanged<VoiceNoteRecording> onRecorded,
  }) : _isPreparing = isPreparing,
       _isRecording = isRecording,
       _onRecorded = onRecorded;

  final VoidCallback start;
  final VoidCallback startTranscription;
  final Widget? transcriptionPanel;
  final bool isDictating;
  final ComposerTranscription transcription;
  final VoidCallback onKeyboardHidden;
  final VoidCallback onDraftIdentityChanged;

  /// The recording whose transcript the editor's next draft snapshot carries.
  final ObjectRef<String?> insertedTranscriptionId;

  final ValueNotifier<bool> _isPreparing;
  final ValueNotifier<bool> _isRecording;
  final ValueChanged<VoiceNoteRecording> _onRecorded;

  bool get isPreparing => _isPreparing.value;
  bool get isRecording => _isRecording.value;
  Widget? get recorder =>
      transcriptionPanel ??
      (isRecording
          ? VoiceNoteComposerRecorder(
              isDictation: isDictating,
              onCancel: () => _isRecording.value = false,
              onRecorded: _onRecorded,
              onStopping: isDictating ? transcription.stopping : null,
              onStopFailed: isDictating ? transcription.stopFailed : null,
            )
          : transcription.pendingStop.value
          ? Semantics(liveRegion: true, child: const Text('Saving recording…'))
          : null);
}

bool _voiceNoteFullWidth(
  _ComposerVoiceNote voiceNote,
  List<_PendingAttachment> attachments,
) =>
    voiceNote.transcriptionPanel != null ||
    voiceNote.transcription.pendingStop.value ||
    voiceNote.isPreparing ||
    voiceNote.isRecording ||
    attachments.any((item) => item.kind == _PendingAttachmentKind.voiceNote);

_ComposerVoiceNote _useComposerVoiceNote({
  required BuildContext context,
  required WidgetRef ref,
  required TextEditingController controller,
  required String scope,
  required String draftKey,
  required ValueNotifier<_InputTool?> inputTool,
  required FocusNode focusNode,
  required ValueNotifier<bool> isComposerExpanded,
  required ValueNotifier<bool> showFormatting,
  required ValueNotifier<_AttachmentSurface> attachmentSurface,
  required ValueNotifier<String?> uploadError,
  required ObjectRef<int> draftRevision,
  required ValueNotifier<List<_PendingAttachment>> attachments,
}) {
  final owner = useRef(scope)..value = scope;
  final isPreparing = useState(false);
  final isRecording = useState(false);
  final dictating = useState(false);
  final availabilityCheck = useRef<Object?>(null);
  final insertedTranscriptionId = useRef<String?>(null);
  final transcript = useComposerTranscription(
    context: context,
    ref: ref,
    scope: scope,
    draftKey: draftKey,
    onText: (text, recordingId) {
      final existing = controller.text;
      final separator =
          existing.isEmpty || existing.endsWith(' ') || existing.endsWith('\n')
          ? ''
          : ' ';
      final result = '$existing$separator$text';
      // The editor's own draft snapshot saves this text, the mentions bound in
      // the editor and the recording's identity together, in one write.
      insertedTranscriptionId.value = recordingId;
      try {
        controller.value = TextEditingValue(
          text: result,
          selection: TextSelection.collapsed(offset: result.length),
        );
      } finally {
        insertedTranscriptionId.value = null;
      }
      isComposerExpanded.value = true;
      focusNode.requestFocus();
    },
  );

  final resetForDraftIdentityChange = useCallback(() {
    isPreparing.value = false;
    isRecording.value = false;
    dictating.value = false;
    availabilityCheck.value = null;
  }, [isPreparing, isRecording, dictating, availabilityCheck]);

  void beginRecording() {
    if (!isPreparing.value) return;
    isPreparing.value = false;
    isRecording.value = true;
  }

  void start({bool transcribe = false}) {
    if (transcript.pendingStop.value ||
        transcript.panel != null ||
        isPreparing.value ||
        isRecording.value) {
      return;
    }
    availabilityCheck.value = null;
    dictating.value = transcribe;
    if (attachments.value.isNotEmpty) {
      uploadError.value = 'A voice note must be the only attachment.';
      return;
    }
    attachmentSurface.value = _AttachmentSurface.closed;
    showFormatting.value = false;
    isComposerExpanded.value = false;
    _dismissComposerKeyboard(focusNode);
    if (ref.read(huddleSessionProvider).isInSession) {
      uploadError.value = 'Leave the Huddle before recording a voice note.';
      return;
    }
    uploadError.value = null;
    draftRevision.value += 1;
    isPreparing.value = true;
    if (View.of(context).viewInsets.bottom == 0) beginRecording();
  }

  Future<void> startTranscription() async {
    if (availabilityCheck.value != null || transcript.pendingStop.value) return;
    final request = Object();
    availabilityCheck.value = request;
    var available = false;
    try {
      available = await transcript.isAvailable().timeout(
        const Duration(seconds: 5),
      );
    } catch (_) {
      // A missing or unresponsive native recognizer cannot handle dictation.
    }
    if (!context.mounted || availabilityCheck.value != request) return;
    availabilityCheck.value = null;
    if (controller.text.isNotEmpty) return;
    if (!available) {
      uploadError.value =
          'Dictation isn’t available on this device or language.';
      return;
    }
    inputTool.value = null;
    start(transcribe: true);
  }

  void complete(VoiceNoteRecording recording) {
    draftRevision.value += 1;
    uploadError.value = null;
    attachments.value = [
      ...attachments.value,
      _PendingAttachment(
        file: recording.file,
        kind: _PendingAttachmentKind.voiceNote,
        deleteAfterUse: true,
        duration: recording.duration,
        waveform: recording.waveform,
      ),
    ];
    isRecording.value = false;
  }

  return _ComposerVoiceNote(
    start: start,
    startTranscription: () => unawaited(startTranscription()),
    transcriptionPanel: transcript.panel,
    isDictating: dictating.value,
    transcription: transcript,
    onKeyboardHidden: beginRecording,
    onDraftIdentityChanged: resetForDraftIdentityChange,
    insertedTranscriptionId: insertedTranscriptionId,
    isPreparing: isPreparing,
    isRecording: isRecording,
    onRecorded: dictating.value
        ? (recording) {
            transcript.complete(recording);
            if (context.mounted && owner.value == scope) {
              isRecording.value = false;
            }
          }
        : complete,
  );
}
