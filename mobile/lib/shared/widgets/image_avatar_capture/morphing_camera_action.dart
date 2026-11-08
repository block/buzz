part of '../image_avatar_capture.dart';

class _MorphingCameraAction extends StatelessWidget {
  const _MorphingCameraAction({
    required this.controlKey,
    required this.width,
    required this.icon,
    required this.iosIcon,
    required this.label,
    required this.transitionLabel,
    required this.transitionLabelMaxWidth,
    required this.transitionProgress,
    required this.showEnabledAppearance,
    required this.semanticLabel,
    required this.onTap,
  });

  final Key controlKey;
  final double width;
  final IconData icon;
  final IosGlassNavigationIcon iosIcon;
  final String? label;
  final String? transitionLabel;
  final double transitionLabelMaxWidth;
  final double transitionProgress;
  final bool showEnabledAppearance;
  final String semanticLabel;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    Widget control;
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      control = IgnorePointer(
        ignoring: onTap == null,
        child: IosGlassNavigationButton(
          icon: iosIcon,
          label: label,
          semanticLabel: semanticLabel,
          onPressed: onTap ?? (showEnabledAppearance ? () {} : null),
          width: width,
          height: _cameraControlSize,
          controlSize: _cameraControlSize,
          fillWidth: true,
          foregroundColor: context.colors.onSurface,
        ),
      );
    } else {
      final dimmed = onTap == null && !showEnabledAppearance;
      control = Semantics(
        label: semanticLabel,
        button: true,
        enabled: onTap != null,
        child: ExcludeSemantics(
          child: Material(
            color: context.colors.surfaceContainerHighest,
            shape: const StadiumBorder(),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              onTap: onTap,
              child: SizedBox(
                width: width,
                height: _cameraControlSize,
                child: Center(
                  child: AnimatedSwitcher(
                    duration: MediaQuery.disableAnimationsOf(context)
                        ? Duration.zero
                        : const Duration(milliseconds: 120),
                    switchInCurve: Curves.easeOutCubic,
                    switchOutCurve: Curves.easeOutCubic,
                    child: label == null
                        ? Icon(
                            icon,
                            key: ValueKey('camera-action-icon-${iosIcon.name}'),
                            size: 26,
                            color: dimmed
                                ? context.colors.onSurface.withValues(
                                    alpha: 0.38,
                                  )
                                : context.colors.onSurface,
                          )
                        : Text(
                            label!,
                            key: ValueKey(label),
                            maxLines: 1,
                            style: context.textTheme.labelMedium?.copyWith(
                              color: dimmed
                                  ? context.colors.onSurface.withValues(
                                      alpha: 0.38,
                                    )
                                  : context.colors.onSurface,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    }

    final labelProgress = transitionProgress.clamp(0.0, 1.0);
    return Stack(
      clipBehavior: Clip.none,
      children: [
        Positioned(
          key: controlKey,
          left: 0,
          right: 0,
          top: (_cameraControlRailHeight - _cameraControlSize) / 2,
          height: _cameraControlSize,
          child: Transform.translate(
            offset: Offset(0, 1.5 * labelProgress),
            child: control,
          ),
        ),
        if (transitionLabel != null)
          Positioned(
            left: 0,
            right: 0,
            top: _cameraControlRailHeight - 20,
            height: 20,
            child: Opacity(
              key: ValueKey('camera-transition-label-${transitionLabel!}'),
              opacity: labelProgress,
              child: Transform.translate(
                offset: Offset(0, 2 * (1 - labelProgress)),
                child: OverflowBox(
                  minWidth: 0,
                  maxWidth: transitionLabelMaxWidth,
                  maxHeight: 20,
                  child: Text(
                    transitionLabel!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}
