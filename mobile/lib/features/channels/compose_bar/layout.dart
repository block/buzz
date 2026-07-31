part of '../compose_bar.dart';

class _ComposeBarLayout extends StatelessWidget {
  final List<BlobDescriptor> attachments;
  final int uploadingCount;
  final ValueChanged<String> onRemoveAttachment;
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
  final double expansionValue;
  final double expansionProgress;
  final bool formattingOpen;
  final VoidCallback onCloseFormatting;
  final Duration motionDuration;
  final void Function(String prefix, [String? suffix]) onFormat;
  final VoidCallback onMention;
  final VoidCallback onChannel;
  final VoidCallback onEmoji;
  final VoidCallback onOpenFormatting;
  final bool hasPendingUploads;
  final bool isSending;

  const _ComposeBarLayout({
    required this.attachments,
    required this.uploadingCount,
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
    required this.expansionValue,
    required this.expansionProgress,
    required this.formattingOpen,
    required this.onCloseFormatting,
    required this.motionDuration,
    required this.onFormat,
    required this.onMention,
    required this.onChannel,
    required this.onEmoji,
    required this.onOpenFormatting,
    required this.hasPendingUploads,
    required this.isSending,
  });

  @override
  Widget build(BuildContext context) {
    return _DragDownToDismissKeyboard(child: _buildBar(context));
  }

  Widget _buildBar(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.dialog),
        border: Border.all(
          color: Colors.black.withValues(alpha: 0.04),
          width: 1,
        ),
      ),
      padding: const EdgeInsets.all(Grid.xxs),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (attachments.isNotEmpty || hasPendingUploads) ...[
            _AttachmentStrip(
              attachments: attachments,
              uploadingCount: uploadingCount,
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
            TextField(
              controller: controller,
              focusNode: focusNode,
              textInputAction: TextInputAction.send,
              contextMenuBuilder: contextMenuBuilder,
              contentInsertionConfiguration: ContentInsertionConfiguration(
                allowedMimeTypes: _pastedImageMimeTypes,
                onContentInserted: onContentInserted,
              ),
              onSubmitted: (_) => onSend(),
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
                      onTap: onExpand,
                      child: Padding(
                        padding: const EdgeInsets.symmetric(
                          vertical: Grid.half,
                        ),
                        child: Align(
                          alignment: Alignment.centerLeft,
                          child: Text(
                            resolvedHint,
                            style: context.textTheme.bodyLarge?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ClipRect(
            child: Align(
              alignment: Alignment.topCenter,
              heightFactor: expansionValue,
              child: IgnorePointer(
                ignoring: expansionValue < 0.98,
                child: Opacity(
                  opacity: expansionProgress,
                  child: Transform.translate(
                    offset: Offset(0, Grid.xxs * (1 - expansionProgress)),
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
                                layoutBuilder:
                                    (currentChild, previousChildren) => Stack(
                                      alignment: Alignment.centerLeft,
                                      children: [
                                        ...previousChildren,
                                        ?currentChild,
                                      ],
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
                                            isDisabled: hasPendingUploads,
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
                ),
              ),
            ),
          ),
        ],
      ),
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
