part of '../compose_bar.dart';

enum _DictationPhase { idle, preparing, recording, transcribing }

/// Composer dictation: record with the voice-note recorder, transcribe on the
/// relay, then insert the text at the caret. It never sends.
///
/// State is read lazily, like [_ComposerVoiceNote]: the draft lifecycle resets
/// dictation from an effect that runs later in the same build, so values
/// copied when the hook ran would be stale for the rest of that build.
class _ComposerDictation {
  const _ComposerDictation({
    required ValueNotifier<_DictationPhase> phase,
    required Widget? Function() recorder,
    required Widget? Function() button,
    required this.startVoiceNote,
    required this.onKeyboardHidden,
    required this.onDraftIdentityChanged,
  }) : _phase = phase,
       _recorder = recorder,
       _button = button;

  final ValueNotifier<_DictationPhase> _phase;
  final Widget? Function() _recorder;
  final Widget? Function() _button;

  /// Starts a voice note unless dictation holds the microphone or a request.
  final VoidCallback startVoiceNote;

  /// Starts a prepared recording once the keyboard is gone, as voice notes do.
  final VoidCallback onKeyboardHidden;

  /// Resets voice note and dictation together when the draft changes.
  final VoidCallback onDraftIdentityChanged;

  /// Waiting for the keyboard to go, or recording.
  bool get isCapturing =>
      _phase.value == _DictationPhase.preparing ||
      _phase.value == _DictationPhase.recording;
  bool get isTranscribing => _phase.value == _DictationPhase.transcribing;

  /// The recording row, shown in place of the composer while recording.
  Widget? get recorder => _recorder();

  /// The mic or transcribing control, or null when dictation is unavailable.
  Widget? get button => _button();
}

String _dictationFailureMessage(TranscriptionFailure failure) =>
    switch (failure) {
      TranscriptionFailure.network =>
        'Couldn’t reach the server. Nothing was added; try dictating again.',
      TranscriptionFailure.service =>
        'Transcription failed. Nothing was added; try dictating again.',
      TranscriptionFailure.rateLimited =>
        'Too many dictations. Try again in a minute.',
      TranscriptionFailure.tooLarge =>
        'This recording is too long to transcribe. Try a shorter one.',
    };

/// Inserts [transcript] at the end of the current selection, or appends it,
/// padding with spaces so it never fuses with neighboring words. Typed text is
/// never replaced.
void _insertDictatedText(
  _MarkdownEditingController controller,
  ObjectRef<bool> isModifyingText,
  String transcript,
) {
  final text = controller.text;
  final selection = controller.selection;
  final offset = selection.isValid
      ? selection.end.clamp(0, text.length)
      : text.length;
  final before = offset > 0 && text[offset - 1].trim().isNotEmpty ? ' ' : '';
  final after = offset < text.length && text[offset].trim().isNotEmpty
      ? ' '
      : '';
  final insertion = '$before$transcript$after';
  isModifyingText.value = true;
  try {
    controller.value = TextEditingValue(
      text: text.replaceRange(offset, offset, insertion),
      selection: TextSelection.collapsed(
        offset: offset + before.length + transcript.length,
      ),
    );
  } finally {
    isModifyingText.value = false;
  }
}

_ComposerDictation _useComposerDictation({
  required BuildContext context,
  required WidgetRef ref,
  required _MarkdownEditingController controller,
  required FocusNode focusNode,
  required ObjectRef<bool> isModifyingText,
  required ValueNotifier<bool> isComposerExpanded,
  required ValueNotifier<bool> showFormatting,
  required ValueNotifier<_AttachmentSurface> attachmentSurface,
  required ValueNotifier<String?> uploadError,
  required List<_PendingAttachment> attachments,
  required _ComposerVoiceNote voiceNote,
}) {
  final capability = ref.watch(transcriptionCapabilityProvider).asData?.value;
  final phase = useState(_DictationPhase.idle);
  // Bumped by every start, stop, cancel and draft change. A transcript only
  // lands while its own generation is still current.
  final generation = useRef(0);
  // Completing it aborts the in-flight request.
  final abortRequest = useRef<Completer<void>?>(null);
  useEffect(
    () =>
        () => abortRequest.value?.complete(),
    const [],
  );

  void announce(String message) {
    if (!context.mounted) return;
    unawaited(
      SemanticsService.sendAnnouncement(
        View.of(context),
        message,
        Directionality.of(context),
      ),
    );
  }

  final cancel = useCallback(() {
    generation.value += 1;
    abortRequest.value?.complete();
    abortRequest.value = null;
    phase.value = _DictationPhase.idle;
  }, [phase]);

  final resetForDraftIdentityChange = voiceNote.onDraftIdentityChanged;
  final onDraftIdentityChanged = useCallback(() {
    resetForDraftIdentityChange();
    cancel();
  }, [resetForDraftIdentityChange, cancel]);

  void beginRecording() {
    if (phase.value != _DictationPhase.preparing) return;
    phase.value = _DictationPhase.recording;
    announce('Dictation recording');
  }

  void start() {
    if (phase.value != _DictationPhase.idle) return;
    if (ref.read(huddleSessionProvider).isInSession) {
      uploadError.value = 'Leave the Huddle before dictating.';
      return;
    }
    attachmentSurface.value = _AttachmentSurface.closed;
    showFormatting.value = false;
    isComposerExpanded.value = false;
    _dismissComposerKeyboard(focusNode);
    uploadError.value = null;
    generation.value += 1;
    phase.value = _DictationPhase.preparing;
    if (View.of(context).viewInsets.bottom == 0) beginRecording();
  }

  void userCancel() {
    cancel();
    announce('Dictation cancelled');
  }

  Future<void> transcribe(VoiceNoteRecording recording) async {
    final current = ++generation.value;
    final abort = abortRequest.value = Completer<void>();
    phase.value = _DictationPhase.transcribing;
    announce('Transcribing');
    String? message;
    try {
      final Uint8List audio;
      try {
        audio = await recording.file.readAsBytes();
      } finally {
        // Audio is never kept: the temporary recording goes once it is read.
        unawaited(deleteDroppedVoiceNoteRecording(recording.file.path));
      }
      final target = capability;
      if (target == null) {
        throw const TranscriptionException(TranscriptionFailure.service);
      }
      final transcript =
          (await ref
                  .read(transcriptionClientProvider)
                  .transcribe(
                    audio,
                    capability: target,
                    cancelled: abort.future,
                  ))
              .trim();
      if (!context.mounted || generation.value != current) return;
      if (transcript.isEmpty) {
        message = 'No speech detected. Nothing was added.';
      } else {
        _insertDictatedText(controller, isModifyingText, transcript);
        announce('Transcript added');
      }
    } on TranscriptionException catch (error) {
      message = _dictationFailureMessage(error.failure);
    } catch (_) {
      message = _dictationFailureMessage(TranscriptionFailure.service);
    }
    if (!context.mounted || generation.value != current) return;
    abortRequest.value = null;
    phase.value = _DictationPhase.idle;
    uploadError.value = message;
    if (message != null) announce(message);
  }

  bool isAvailable() =>
      capability != null &&
      !voiceNote.isPreparing &&
      !attachments.any((item) => item.kind == _PendingAttachmentKind.voiceNote);

  return _ComposerDictation(
    phase: phase,
    recorder: () => phase.value == _DictationPhase.recording
        ? VoiceNoteComposerRecorder(
            key: const ValueKey('composer-dictation-recorder'),
            onCancel: userCancel,
            onRecorded: (recording) => unawaited(transcribe(recording)),
            discardTooltip: 'Cancel dictation',
            stopTooltip: 'Stop and transcribe',
          )
        : null,
    button: () => switch (phase.value) {
      _DictationPhase.transcribing => _DictationButton(
        isTranscribing: true,
        onPressed: userCancel,
      ),
      _DictationPhase.idle when isAvailable() => _DictationButton(
        isTranscribing: false,
        onPressed: start,
      ),
      _ => null,
    },
    startVoiceNote: () {
      if (phase.value == _DictationPhase.idle) {
        voiceNote.start();
      } else {
        uploadError.value = 'Finish dictating before recording a voice note.';
      }
    },
    onKeyboardHidden: beginRecording,
    onDraftIdentityChanged: onDraftIdentityChanged,
  );
}

class _DictationButton extends StatelessWidget {
  const _DictationButton({
    required this.isTranscribing,
    required this.onPressed,
  });

  final bool isTranscribing;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    // The IconButton's tooltip is the control's only label.
    return SizedBox(
      key: const ValueKey('composer-dictation-button'),
      width: 36,
      height: 36,
      child: IconButton(
        tooltip: isTranscribing ? 'Cancel transcription' : 'Dictate',
        onPressed: () => _runComposerAction(onPressed),
        padding: EdgeInsets.zero,
        visualDensity: VisualDensity.compact,
        icon: isTranscribing
            ? ExcludeSemantics(
                child: BuzzLoadingIndicator(
                  size: 18,
                  color: context.colors.onSurfaceVariant,
                ),
              )
            : Icon(
                BuzzIcons.mic,
                size: 20,
                color: context.colors.onSurfaceVariant,
              ),
      ),
    );
  }
}
