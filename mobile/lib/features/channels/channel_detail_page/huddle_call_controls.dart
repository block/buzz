part of '../channel_detail_page.dart';

class _HuddleCallControls extends StatelessWidget {
  const _HuddleCallControls({
    required this.isMuted,
    required this.isSpeakerEnabled,
    required this.onToggleMute,
    required this.onToggleSpeaker,
    required this.onReact,
  });

  final bool isMuted;
  final bool isSpeakerEnabled;
  final VoidCallback onToggleMute;
  final VoidCallback onToggleSpeaker;
  final VoidCallback onReact;

  @override
  Widget build(BuildContext context) {
    return Padding(
      key: const ValueKey('huddle-call-controls'),
      padding: const EdgeInsets.only(top: Grid.xxs),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          _HuddleRoundControl(
            key: const ValueKey('huddle-speaker-toggle'),
            tooltip: isSpeakerEnabled ? 'Use earpiece' : 'Use speaker',
            icon: LucideIcons.volume2,
            foregroundColor: isSpeakerEnabled
                ? context.colors.onPrimary
                : context.colors.onSurface,
            backgroundColor: isSpeakerEnabled
                ? context.colors.primary
                : context.colors.surfaceContainerHighest,
            dimension: 80,
            toggled: isSpeakerEnabled,
            useHapticFeedback: true,
            onPressed: onToggleSpeaker,
          ),
          const SizedBox(width: Grid.sm),
          _HuddleRoundControl(
            key: const ValueKey('huddle-mute-toggle'),
            tooltip: isMuted ? 'Unmute' : 'Mute',
            icon: isMuted ? LucideIcons.micOff : LucideIcons.mic,
            foregroundColor: isMuted
                ? context.colors.onSurface
                : context.colors.onPrimary,
            backgroundColor: isMuted
                ? context.colors.surfaceContainerHighest
                : context.colors.primary,
            dimension: 80,
            toggled: isMuted,
            useHapticFeedback: true,
            onPressed: onToggleMute,
          ),
          const SizedBox(width: Grid.sm),
          _HuddleRoundControl(
            key: const ValueKey('huddle-emoji-reactions'),
            tooltip: 'Emoji reactions',
            icon: LucideIcons.smilePlus,
            foregroundColor: context.colors.onSurface,
            backgroundColor: context.colors.surfaceContainerHighest,
            dimension: 80,
            useHapticFeedback: true,
            onPressed: onReact,
          ),
        ],
      ),
    );
  }
}

class _HuddleRoundControl extends StatelessWidget {
  const _HuddleRoundControl({
    super.key,
    required this.tooltip,
    required this.icon,
    required this.foregroundColor,
    required this.backgroundColor,
    required this.onPressed,
    this.dimension = 64,
    this.showTooltip = true,
    this.toggled,
    this.useHapticFeedback = false,
  });

  final String tooltip;
  final IconData icon;
  final Color foregroundColor;
  final Color backgroundColor;
  final VoidCallback? onPressed;
  final double dimension;
  final bool showTooltip;
  final bool? toggled;
  final bool useHapticFeedback;

  @override
  Widget build(BuildContext context) {
    final effectiveOnPressed = onPressed == null
        ? null
        : () {
            if (useHapticFeedback) {
              unawaited(HapticFeedback.selectionClick());
            }
            onPressed!();
          };

    return Semantics(
      label: tooltip,
      button: true,
      enabled: onPressed != null,
      toggled: toggled,
      onTap: effectiveOnPressed,
      child: ExcludeSemantics(
        child: SizedBox.square(
          dimension: dimension,
          child: IconButton(
            tooltip: showTooltip ? tooltip : null,
            onPressed: effectiveOnPressed,
            style: IconButton.styleFrom(
              foregroundColor: foregroundColor,
              backgroundColor: backgroundColor,
              disabledForegroundColor: foregroundColor.withValues(alpha: 0.5),
              disabledBackgroundColor: backgroundColor.withValues(alpha: 0.5),
            ),
            icon: Icon(icon, size: 28),
          ),
        ),
      ),
    );
  }
}
