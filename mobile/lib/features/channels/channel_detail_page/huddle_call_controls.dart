part of '../channel_detail_page.dart';

class _HuddleCallControls extends StatelessWidget {
  const _HuddleCallControls({
    required this.isMuted,
    this.inRail = false,
    required this.isSpeakerEnabled,
    required this.onToggleMute,
    required this.onToggleSpeaker,
    required this.onReact,
  });

  final bool inRail;
  final bool isMuted;
  final bool isSpeakerEnabled;
  final VoidCallback onToggleMute;
  final VoidCallback onToggleSpeaker;
  final VoidCallback onReact;

  @override
  Widget build(BuildContext context) {
    final microphone = _HuddleRoundControl(
      useGlass: true,
      key: const ValueKey('huddle-mute-toggle'),
      tooltip: isMuted ? 'Unmute' : 'Mute',
      glassSemanticLabel: isMuted ? 'Microphone off' : 'Microphone on',
      icon: isMuted ? BuzzIcons.micOff : BuzzIcons.mic,
      foregroundColor: isMuted
          ? context.colors.onSurface
          : context.colors.onPrimary,
      backgroundColor: isMuted
          ? context.colors.surfaceContainerHighest
          : context.colors.primary,
      dimension: inRail ? _huddleRailControlSize : 80,
      toggled: isMuted,
      glassSelected: !isMuted,
      useHapticFeedback: true,
      onPressed: onToggleMute,
    );
    final speaker = _HuddleRoundControl(
      useGlass: true,
      key: const ValueKey('huddle-speaker-toggle'),
      tooltip: isSpeakerEnabled ? 'Use earpiece' : 'Use speaker',
      glassSemanticLabel: isSpeakerEnabled ? 'Speaker on' : 'Speaker off',
      icon: BuzzIcons.volume2,
      foregroundColor: isSpeakerEnabled
          ? context.colors.onPrimary
          : context.colors.onSurface,
      backgroundColor: isSpeakerEnabled
          ? context.colors.primary
          : context.colors.surfaceContainerHighest,
      dimension: inRail ? _huddleRailControlSize : 80,
      toggled: isSpeakerEnabled,
      useHapticFeedback: true,
      onPressed: onToggleSpeaker,
    );
    return Padding(
      key: const ValueKey('huddle-call-controls'),
      padding: const EdgeInsets.only(top: Grid.xxs),
      child: Flex(
        direction: inRail ? Axis.vertical : Axis.horizontal,
        mainAxisSize: MainAxisSize.min,
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          inRail ? microphone : speaker,
          SizedBox(
            width: inRail ? 0 : Grid.sm,
            height: inRail ? _huddleRailControlGap : 0,
          ),
          inRail ? speaker : microphone,
          // Keep reactions available for a future return to the call controls.
          Visibility(
            visible: false,
            child: Padding(
              padding: EdgeInsets.only(
                left: inRail ? 0 : Grid.sm,
                top: inRail ? _huddleRailControlGap : 0,
              ),
              child: _HuddleRoundControl(
                useGlass: true,
                key: const ValueKey('huddle-emoji-reactions'),
                tooltip: 'Emoji reactions',
                icon: BuzzIcons.smilePlus,
                foregroundColor: context.colors.onSurface,
                backgroundColor: context.colors.surfaceContainerHighest,
                dimension: inRail ? _huddleRailControlSize : 80,
                useHapticFeedback: true,
                onPressed: onReact,
              ),
            ),
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
    this.useGlass = false,
    this.showTooltip = true,
    this.toggled,
    this.glassSelected,
    this.glassSemanticLabel,
    this.useHapticFeedback = false,
  });

  final String tooltip;
  final IconData icon;
  final Color foregroundColor;
  final Color backgroundColor;
  final VoidCallback? onPressed;
  final double dimension;
  static const iconSize = 28.0;
  final bool useGlass;
  final bool showTooltip;
  final bool? toggled;
  final bool? glassSelected;

  /// The native glass button marks [glassSelected] with UIKit's selected
  /// trait, so its label names the state rather than the action.
  final String? glassSemanticLabel;
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

    if (useGlass && defaultTargetPlatform == TargetPlatform.iOS) {
      final selected = glassSelected ?? toggled ?? false;
      final tint = icon == BuzzIcons.phoneOff
          ? context.colors.error
          : selected
          ? context.colors.primary
          : context.colors.onSurface;
      return IosGlassNavigationButton(
        icon: IosGlassNavigationIcon.close,
        label: '', // Flutter retains the existing Tabler glyph.
        semanticLabel: glassSemanticLabel ?? tooltip,
        onPressed: effectiveOnPressed,
        width: dimension,
        height: dimension,
        controlSize: dimension,
        foregroundColor: tint,
        isSelected: selected,
        artwork: Icon(
          icon,
          size: iconSize,
          color: onPressed == null
              ? tint.withValues(alpha: 0.5)
              : selected
              ? Theme.of(context).brightness == Brightness.dark
                    ? Colors.black
                    : Colors.white
              : tint,
        ),
      );
    }

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
            icon: Icon(icon, size: iconSize),
          ),
        ),
      ),
    );
  }
}
