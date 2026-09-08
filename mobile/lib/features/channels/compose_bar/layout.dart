part of '../compose_bar.dart';

class _ComposeBarLayout extends HookWidget {
  final Widget? voiceNoteRecorder;
  final List<_PendingAttachment> attachments;
  final ValueChanged<int> onRemoveAttachment;
  final String? uploadError;
  final bool isExpanded;
  final TextEditingController controller;
  final FocusNode focusNode;
  final EditableTextContextMenuBuilder contextMenuBuilder;
  final ValueChanged<KeyboardInsertedContent> onContentInserted;
  final VoidCallback onSend;
  final String resolvedHint;
  final _AttachmentSurface attachmentSurface;
  final ValueChanged<BuildContext> onAttachmentTap;
  final VoidCallback onExpand;
  final Animation<double> expansionAnimation;
  final bool formattingOpen;
  final VoidCallback onCloseFormatting;
  final Duration motionDuration;
  final Duration resizeDuration;
  final void Function(String prefix, [String? suffix]) onFormat;
  final VoidCallback onMention;
  final VoidCallback onChannel;
  final VoidCallback onEmoji;
  final VoidCallback onOpenFormatting;
  final bool canSend;
  final bool hasPendingUploads;
  final bool isSending;

  const _ComposeBarLayout({
    required this.voiceNoteRecorder,
    required this.attachments,
    required this.onRemoveAttachment,
    required this.uploadError,
    required this.isExpanded,
    required this.controller,
    required this.focusNode,
    required this.contextMenuBuilder,
    required this.onContentInserted,
    required this.onSend,
    required this.resolvedHint,
    required this.attachmentSurface,
    required this.onAttachmentTap,
    required this.onExpand,
    required this.expansionAnimation,
    required this.formattingOpen,
    required this.onCloseFormatting,
    required this.motionDuration,
    required this.resizeDuration,
    required this.onFormat,
    required this.onMention,
    required this.onChannel,
    required this.onEmoji,
    required this.onOpenFormatting,
    required this.canSend,
    required this.hasPendingUploads,
    required this.isSending,
  });

  @override
  Widget build(BuildContext context) {
    final recordingTransition = useAnimationController(
      duration: motionDuration,
      reverseDuration: MediaQuery.disableAnimationsOf(context)
          ? Duration.zero
          : const Duration(milliseconds: 140),
      initialValue: voiceNoteRecorder == null ? 0 : 1,
    );
    useEffect(() {
      if (voiceNoteRecorder == null) {
        recordingTransition.reverse();
      } else {
        recordingTransition.forward();
      }
      return null;
    }, [voiceNoteRecorder != null, motionDuration]);
    return _DragDownToDismissKeyboard(
      child: _buildBar(context, recordingTransition),
    );
  }

  Widget _buildBar(
    BuildContext context,
    Animation<double> recordingTransition,
  ) {
    final trimmedDraft = controller.text.trim();
    final collapsedText = trimmedDraft.isEmpty
        ? resolvedHint
        : trimmedDraft.replaceAll(RegExp(r'\s+'), ' ');
    final hasVoiceNoteAttachment = attachments.any(
      (attachment) => attachment.kind == _PendingAttachmentKind.voiceNote,
    );
    final composerContent = Column(
      key: ValueKey(
        hasVoiceNoteAttachment
            ? 'composer-voice-note-preview-content'
            : 'composer-standard-content',
      ),
      mainAxisSize: MainAxisSize.min,
      children: [
        if (attachments.isNotEmpty) ...[
          _AttachmentStrip(
            attachments: attachments,
            onRemove: onRemoveAttachment,
          ),
          const SizedBox(height: Grid.xxs),
        ],
        if (uploadError case final error?) ...[
          Align(
            alignment: Alignment.centerLeft,
            child: Text(
              error,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.error,
              ),
            ),
          ),
          const SizedBox(height: Grid.xxs),
        ],
        // Keep the default state out of the focus system entirely so
        // restored native focus cannot expand a newly opened channel.
        if (isExpanded)
          resizeDuration == Duration.zero
              ? KeyedSubtree(
                  key: const ValueKey('composer-text-height-motion'),
                  child: _buildTextField(context),
                )
              : AnimatedSize(
                  key: const ValueKey('composer-text-height-motion'),
                  alignment: Alignment.topCenter,
                  duration: resizeDuration,
                  curve: Curves.easeOutCubic,
                  child: _buildTextField(context),
                )
        else
          Row(
            children: [
              _AttachmentTrigger(
                surface: attachmentSurface,
                formattingOpen: false,
                onTap: onAttachmentTap,
              ),
              const SizedBox(width: Grid.xxs),
              Expanded(
                child: Semantics(
                  button: true,
                  label: resolvedHint,
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: () => _runComposerAction(onExpand),
                    child: Padding(
                      padding: const EdgeInsets.symmetric(vertical: Grid.half),
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: Text(
                          collapsedText,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: context.textTheme.bodyLarge?.copyWith(
                            color: trimmedDraft.isEmpty
                                ? context.colors.onSurfaceVariant
                                : context.colors.onSurface,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: Grid.xxs),
              _SendButton(
                isDisabled: !canSend || hasPendingUploads,
                isSending: isSending,
                onTap: onSend,
              ),
            ],
          ),
        _ExpandedComposerActionsMotion(
          animation: expansionAnimation,
          isExpanded: isExpanded,
          child: Column(
            children: [
              const SizedBox(height: Grid.xxs),
              Row(
                children: [
                  _AttachmentTrigger(
                    surface: attachmentSurface,
                    formattingOpen: formattingOpen,
                    onTap: (triggerContext) {
                      if (formattingOpen) {
                        onCloseFormatting();
                      } else {
                        onAttachmentTap(triggerContext);
                      }
                    },
                  ),
                  const SizedBox(width: Grid.half),
                  Expanded(
                    child: AnimatedSwitcher(
                      duration: motionDuration,
                      switchInCurve: Curves.easeOutCubic,
                      switchOutCurve: Curves.easeInCubic,
                      layoutBuilder: (currentChild, previousChildren) => Stack(
                        alignment: Alignment.centerLeft,
                        children: [...previousChildren, ?currentChild],
                      ),
                      child: formattingOpen
                          ? _FormattingToolbar(onFormat: onFormat)
                          : Row(
                              key: const ValueKey('standard-actions'),
                              children: [
                                _ComposeAction(
                                  icon: LucideIcons.atSign,
                                  onTap: onMention,
                                ),
                                _ComposeAction(
                                  icon: LucideIcons.hash,
                                  onTap: onChannel,
                                ),
                                _ComposeAction(
                                  icon: LucideIcons.smilePlus,
                                  onTap: onEmoji,
                                ),
                                _ComposeAction(
                                  icon: LucideIcons.aLargeSmall,
                                  onTap: onOpenFormatting,
                                ),
                                const Spacer(),
                                _SendButton(
                                  isDisabled: !canSend || hasPendingUploads,
                                  isSending: isSending,
                                  onTap: onSend,
                                ),
                              ],
                            ),
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ],
    );
    final contentMotionDuration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 140);
    final content = ClipRect(
      child: AnimatedSwitcher(
        key: const ValueKey('composer-content-morph'),
        duration: contentMotionDuration,
        switchInCurve: Curves.easeOutCubic,
        switchOutCurve: Curves.easeOutCubic,
        layoutBuilder: (currentChild, previousChildren) => Stack(
          alignment: Alignment.bottomCenter,
          children: [...previousChildren, ?currentChild],
        ),
        transitionBuilder: (child, animation) => FadeTransition(
          opacity: animation,
          child: SizeTransition(
            sizeFactor: animation,
            axisAlignment: 1,
            child: child,
          ),
        ),
        child: voiceNoteRecorder == null
            ? composerContent
            : KeyedSubtree(
                key: const ValueKey('composer-voice-note-content'),
                child: voiceNoteRecorder!,
              ),
      ),
    );
    return AnimatedBuilder(
      animation: Listenable.merge([expansionAnimation, recordingTransition]),
      child: content,
      builder: (context, child) {
        final progress = expansionAnimation.value.clamp(0.0, 1.0).toDouble();
        final composerRadius = Radii.dialog + Grid.quarter * (1 - progress);
        final radius = BorderRadius.lerp(
          BorderRadius.circular(composerRadius),
          BorderRadius.circular(Radii.full),
          Curves.easeInOutCubic.transform(recordingTransition.value),
        )!;
        final usesIosConcentricSurface =
            defaultTargetPlatform == TargetPlatform.iOS;
        final voiceNoteInsetProgress = hasVoiceNoteAttachment
            ? 1.0
            : recordingTransition.value;
        final composer = Container(
          key: const ValueKey('composer-surface'),
          decoration: BoxDecoration(
            color: usesIosConcentricSurface
                ? Colors.transparent
                : context.colors.surfaceContainerHighest,
            borderRadius: radius,
            border: Border.all(
              color: Colors.black.withValues(alpha: 0.04),
              width: 1,
            ),
          ),
          padding: EdgeInsets.all(
            Grid.xxs + Grid.half * voiceNoteInsetProgress,
          ),
          child: child,
        );
        if (!usesIosConcentricSurface) return composer;
        return ConcentricSheetSurface(
          key: const ValueKey('composer-ios-concentric-surface'),
          enabled: true,
          usesGlass: true,
          color: context.colors.surfaceContainerHighest,
          padding: EdgeInsets.zero,
          providesSheetSurface: false,
          minimumRadius: radius.topLeft.x,
          contentClipRadius: radius.topLeft.x,
          child: composer,
        );
      },
    );
  }

  Widget _buildTextField(BuildContext context) {
    return TextField(
      controller: controller,
      focusNode: focusNode,
      keyboardType: TextInputType.multiline,
      textInputAction: TextInputAction.newline,
      contextMenuBuilder: contextMenuBuilder,
      // Flutter's Cupertino magnifier rebuilds its overlay on every
      // selection-handle update. Keep the iOS handles and native edit menu,
      // but let the handles track the finger directly here.
      magnifierConfiguration: defaultTargetPlatform == TargetPlatform.iOS
          ? TextMagnifierConfiguration.disabled
          : null,
      contentInsertionConfiguration: ContentInsertionConfiguration(
        allowedMimeTypes: _pastedImageMimeTypes,
        onContentInserted: onContentInserted,
      ),
      minLines: 1,
      maxLines: 5,
      style: context.textTheme.bodyLarge,
      decoration: InputDecoration(
        hintText: resolvedHint,
        hintStyle: context.textTheme.bodyLarge?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
        border: InputBorder.none,
        enabledBorder: InputBorder.none,
        focusedBorder: InputBorder.none,
        contentPadding: const EdgeInsets.symmetric(
          horizontal: Grid.half,
          vertical: Grid.half,
        ),
        isDense: true,
      ),
    );
  }
}

class _ExpandedComposerActionsMotion extends StatelessWidget {
  final Animation<double> animation;
  final bool isExpanded;
  final Widget child;

  const _ExpandedComposerActionsMotion({
    required this.animation,
    required this.isExpanded,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: animation,
      child: child,
      builder: (context, child) {
        final value = animation.value;
        final progress = value.clamp(0.0, 1.0).toDouble();
        return ClipRect(
          child: Align(
            alignment: Alignment.topCenter,
            heightFactor: value,
            child: IgnorePointer(
              ignoring: !isExpanded,
              child: Opacity(
                opacity: progress,
                child: Transform.translate(
                  offset: Offset(0, Grid.xxs * (1 - progress)),
                  child: child,
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// Drag the compose bar downward to put the keyboard away.
///
/// Continues the gesture the message list starts: once your finger reaches the
/// composer, keep pulling down and the keyboard goes with it. Uses a raw
/// [Listener] rather than a `GestureDetector` on purpose — a gesture recognizer
/// here would enter the arena against the `TextField` and could steal taps,
/// caret placement, and selection drags. A [Listener] only observes.
class _DragDownToDismissKeyboard extends HookWidget {
  final Widget child;

  const _DragDownToDismissKeyboard({required this.child});

  @override
  Widget build(BuildContext context) {
    // A ref, not state: pointer travel must not rebuild the composer, which
    // would churn the TextField mid-gesture.
    final downwardTravel = useRef(0.0);
    final startedInEditable = useRef(false);

    return Listener(
      onPointerDown: (event) {
        downwardTravel.value = 0;
        final hitTest = HitTestResult();
        RendererBinding.instance.hitTestInView(
          hitTest,
          event.position,
          event.viewId,
        );
        startedInEditable.value = hitTest.path.any(
          (entry) => entry.target is RenderEditable,
        );
      },
      onPointerCancel: (_) {
        downwardTravel.value = 0;
        startedInEditable.value = false;
      },
      onPointerUp: (_) {
        downwardTravel.value = 0;
        startedInEditable.value = false;
      },
      onPointerMove: (event) {
        if (startedInEditable.value) return;
        final dy = event.delta.dy;
        if (dy <= 0) {
          downwardTravel.value = 0;
          return;
        }
        downwardTravel.value += dy;
        if (downwardTravel.value < keyboardDismissDragThreshold) return;
        downwardTravel.value = 0;
        dismissKeyboard(context);
      },
      child: child,
    );
  }
}
