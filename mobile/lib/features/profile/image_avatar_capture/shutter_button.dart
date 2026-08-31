part of '../image_avatar_capture.dart';

class _ShutterButton extends StatelessWidget {
  const _ShutterButton({required this.busy, required this.onTap});

  final bool busy;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final isIos = defaultTargetPlatform == TargetPlatform.iOS;
    final content = SizedBox(
      key: const ValueKey('image-camera-shutter-morph'),
      width: _shutterSize,
      height: _shutterSize,
      child: isIos
          ? IosGlassNavigationButton(
              icon: IosGlassNavigationIcon.shutter,
              semanticLabel: 'Take photo',
              onPressed: onTap,
              width: _shutterSize,
              height: _shutterSize,
              controlSize: _shutterSize,
              foregroundColor: context.colors.onSurface,
              isBusy: busy,
            )
          : Material(
              color: context.colors.surfaceContainerHighest,
              shape: const CircleBorder(),
              clipBehavior: Clip.antiAlias,
              child: InkWell(
                key: const ValueKey('image-camera-shutter'),
                onTap: onTap,
                child: Center(
                  child: AnimatedSwitcher(
                    duration: reduceMotion
                        ? Duration.zero
                        : const Duration(milliseconds: 140),
                    switchInCurve: Curves.easeOutCubic,
                    switchOutCurve: Curves.easeOutCubic,
                    child: busy
                        ? BuzzLoadingIndicator(
                            key: const ValueKey('image-camera-capturing'),
                            size: 24,
                            color: context.colors.onSurface,
                            semanticLabel: 'Taking photo',
                          )
                        : Container(
                            key: const ValueKey('image-camera-shutter-icon'),
                            width: _shutterCoreSize,
                            height: _shutterCoreSize,
                            decoration: BoxDecoration(
                              color: context.colors.onSurface,
                              shape: BoxShape.circle,
                              border: Border.all(
                                color: context.colors.surface,
                                width: 1.5,
                              ),
                            ),
                          ),
                  ),
                ),
              ),
            ),
    );
    if (isIos) return content;
    return Semantics(
      label: 'Take photo',
      button: true,
      enabled: onTap != null,
      child: ExcludeSemantics(child: content),
    );
  }
}
