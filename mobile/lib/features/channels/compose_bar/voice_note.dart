part of '../compose_bar.dart';

class _ComposerVoiceNote {
  const _ComposerVoiceNote({
    required this.start,
    required this.onKeyboardHidden,
    required this.onDraftIdentityChanged,
    required ValueNotifier<bool> isPreparing,
    required ValueNotifier<bool> isRecording,
    required ValueChanged<VoiceNoteRecording> onRecorded,
  }) : _isPreparing = isPreparing,
       _isRecording = isRecording,
       _onRecorded = onRecorded;

  final VoidCallback start;
  final VoidCallback onKeyboardHidden;
  final VoidCallback onDraftIdentityChanged;
  final ValueNotifier<bool> _isPreparing;
  final ValueNotifier<bool> _isRecording;
  final ValueChanged<VoiceNoteRecording> _onRecorded;

  bool get isPreparing => _isPreparing.value;
  bool get isRecording => _isRecording.value;
  Widget? get recorder => isRecording
      ? VoiceNoteComposerRecorder(
          onCancel: () => _isRecording.value = false,
          onRecorded: _onRecorded,
        )
      : null;
}

bool _voiceNoteFullWidth(
  _ComposerVoiceNote voiceNote,
  List<_PendingAttachment> attachments,
) =>
    voiceNote.isPreparing ||
    voiceNote.isRecording ||
    attachments.any((item) => item.kind == _PendingAttachmentKind.voiceNote);

_ComposerVoiceNote _useComposerVoiceNote({
  required BuildContext context,
  required WidgetRef ref,
  required FocusNode focusNode,
  required ValueNotifier<bool> isComposerExpanded,
  required ValueNotifier<bool> showFormatting,
  required ValueNotifier<_AttachmentSurface> attachmentSurface,
  required ValueNotifier<String?> uploadError,
  required ObjectRef<int> draftRevision,
  required ValueNotifier<List<_PendingAttachment>> attachments,
}) {
  final isPreparing = useState(false);
  final isRecording = useState(false);

  final resetForDraftIdentityChange = useCallback(() {
    isPreparing.value = false;
    isRecording.value = false;
  }, [isPreparing, isRecording]);

  void beginRecording() {
    if (!isPreparing.value) return;
    isPreparing.value = false;
    isRecording.value = true;
  }

  void start() {
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
    onKeyboardHidden: beginRecording,
    onDraftIdentityChanged: resetForDraftIdentityChange,
    isPreparing: isPreparing,
    isRecording: isRecording,
    onRecorded: complete,
  );
}
