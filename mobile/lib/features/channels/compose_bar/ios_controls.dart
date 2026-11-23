part of '../compose_bar.dart';

/// Separate attachment and send circles flank the iOS text capsule.
class _IosComposerControls extends StatelessWidget {
  const _IosComposerControls({
    required this.child,
    required this.showSend,
    required this.isSending,
    required this.canSend,
    required this.enabled,
    required this.onAttachments,
    required this.attachmentSurface,
    required this.onSend,
    required this.onVoiceNote,
  });
  final _AttachmentSurface attachmentSurface;
  final Widget child;
  final bool showSend, isSending, canSend, enabled;
  final ValueChanged<BuildContext> onAttachments;
  final VoidCallback onSend, onVoiceNote;

  @override
  Widget build(BuildContext context) => TextFieldTapRegion(
    child: Row(
      key: const ValueKey('ios-composer-controls'),
      crossAxisAlignment: CrossAxisAlignment.end,
      children: [
        _AttachmentTrigger(
          key: const ValueKey('ios-composer-plus'),
          size: 40,
          enabled: enabled,
          surface: attachmentSurface,
          formattingOpen: false,
          onTap: onAttachments,
        ),
        const SizedBox(width: 8),
        Expanded(child: child),
        ...[
          const SizedBox(width: 8),
          IosGlassNavigationButton(
            icon: showSend
                ? IosGlassNavigationIcon.send
                : IosGlassNavigationIcon.microphone,
            semanticLabel: showSend ? 'Send' : 'Dictate',
            width: 40,
            height: 40,
            controlSize: 40,
            iconSize: showSend ? 18 : 20,
            isBusy: isSending,
            backgroundColor: showSend ? context.colors.primary : null,
            foregroundColor: showSend
                ? context.colors.onPrimary
                : context.colors.onSurface,
            onPressed: showSend
                ? (canSend && !isSending ? onSend : null)
                : (enabled ? onVoiceNote : null),
          ),
        ],
      ],
    ),
  );
}

class _IosEmptyComposerActions extends StatelessWidget {
  const _IosEmptyComposerActions({
    required this.onEmoji,
    required this.enabled,
  });
  final VoidCallback onEmoji;
  final bool enabled;

  @override
  Widget build(BuildContext context) => TextFieldTapRegion(
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final action in [('Emoji', BuzzIcons.smilePlus, onEmoji)])
          SizedBox(
            width: 28,
            height: 28,
            child: IconButton(
              tooltip: action.$1,
              padding: EdgeInsets.zero,
              icon: Icon(action.$2, size: 20),
              onPressed: enabled ? action.$3 : null,
            ),
          ),
      ],
    ),
  );
}
