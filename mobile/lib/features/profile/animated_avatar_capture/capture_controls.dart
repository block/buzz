part of '../animated_avatar_capture.dart';

const _animatedAvatarPreviewSize = 220.0;
const _animatedAvatarPersonTranslation = 48.0;
const _animatedAvatarShapeSize = 172.0;
const _animatedAvatarShapeYOffset = 20.625;
const _animatedAvatarShapeTranslation = 51.5625;
const _animatedAvatarOutputScale = _outputSize / _animatedAvatarPreviewSize;
const _animatedAvatarOutlineOffsets = [
  (-3, 0),
  (3, 0),
  (0, -3),
  (0, 3),
  (-2, -2),
  (2, -2),
  (-2, 2),
  (2, 2),
];

int _animatedAvatarShapeX(double offset) =>
    (_outputSize / 2 +
            offset *
                _animatedAvatarShapeTranslation *
                _animatedAvatarOutputScale)
        .round();

int _animatedAvatarShapeY(double offset) =>
    (_outputSize / 2 +
            (_animatedAvatarShapeYOffset +
                    offset * _animatedAvatarShapeTranslation) *
                _animatedAvatarOutputScale)
        .round();

int _animatedAvatarShapeRadius(double scale) =>
    (_animatedAvatarShapeSize / 2 * _animatedAvatarOutputScale * scale).round();

class _AnimatedRecordButton extends StatelessWidget {
  const _AnimatedRecordButton({required this.busy, required this.onPressed});

  final bool busy;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    return LayoutBuilder(
      builder: (context, constraints) => Center(
        child: AnimatedContainer(
          key: const ValueKey('animated-avatar-record-morph'),
          duration: reduceMotion
              ? Duration.zero
              : const Duration(milliseconds: 220),
          curve: Curves.easeOutCubic,
          width: busy ? 64 : constraints.maxWidth,
          height: 64,
          child: defaultTargetPlatform == TargetPlatform.iOS
              ? IosGlassNavigationButton(
                  key: const ValueKey('animated-avatar-record'),
                  icon: IosGlassNavigationIcon.shutter,
                  label: busy ? null : 'Record',
                  semanticLabel: 'Record animated avatar',
                  onPressed: busy ? null : onPressed,
                  width: busy ? 64 : constraints.maxWidth,
                  height: 64,
                  controlSize: 64,
                  fillWidth: true,
                  foregroundColor: context.colors.onSurface,
                  isBusy: busy,
                )
              : Material(
                  color: context.colors.onSurface,
                  borderRadius: BorderRadius.circular(Radii.full),
                  clipBehavior: Clip.antiAlias,
                  child: InkWell(
                    key: const ValueKey('animated-avatar-record'),
                    onTap: busy ? null : onPressed,
                    child: Center(
                      child: AnimatedSwitcher(
                        duration: reduceMotion
                            ? Duration.zero
                            : const Duration(milliseconds: 150),
                        switchInCurve: Curves.easeOutCubic,
                        switchOutCurve: Curves.easeOutCubic,
                        child: busy
                            ? BuzzLoadingIndicator(
                                key: const ValueKey(
                                  'animated-avatar-capturing',
                                ),
                                size: 24,
                                color: context.colors.surface,
                                semanticLabel: 'Capturing animated avatar',
                              )
                            : Text(
                                'Record',
                                key: const ValueKey(
                                  'animated-avatar-record-label',
                                ),
                                style: context.textTheme.labelLarge?.copyWith(
                                  color: context.colors.surface,
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
}

class _AspectCorrectCameraPreview extends StatelessWidget {
  const _AspectCorrectCameraPreview({required this.controller});

  final CameraController controller;

  @override
  Widget build(BuildContext context) {
    final aspectRatio = 1 / controller.value.aspectRatio;
    return FittedBox(
      fit: BoxFit.cover,
      clipBehavior: Clip.hardEdge,
      child: SizedBox(
        width: 220 * aspectRatio,
        height: 220,
        child: CameraPreview(controller),
      ),
    );
  }
}

class _AnimatedPersonPreview extends StatelessWidget {
  const _AnimatedPersonPreview({
    required this.bytes,
    required this.offset,
    required this.scale,
    required this.outline,
    required this.outlineColor,
  });

  final Uint8List bytes;
  final Offset offset;
  final double scale;
  final bool outline;
  final Color outlineColor;

  @override
  Widget build(BuildContext context) {
    Widget person({Color? tint, Offset outlineOffset = Offset.zero}) =>
        Transform.translate(
          offset: offset + outlineOffset,
          child: Transform.scale(
            scale: scale,
            child: tint == null
                ? Image.memory(bytes, fit: BoxFit.cover)
                : Opacity(
                    opacity: 0.92,
                    child: ColorFiltered(
                      colorFilter: ColorFilter.mode(tint, BlendMode.srcIn),
                      child: Image.memory(bytes, fit: BoxFit.cover),
                    ),
                  ),
          ),
        );

    return Stack(
      fit: StackFit.expand,
      children: [
        if (outline)
          for (final outlineOffset in const [
            Offset(0, -2.4),
            Offset(2.4, 0),
            Offset(0, 2.4),
            Offset(-2.4, 0),
            Offset(1.7, -1.7),
            Offset(1.7, 1.7),
            Offset(-1.7, 1.7),
            Offset(-1.7, -1.7),
          ])
            person(tint: outlineColor, outlineOffset: outlineOffset),
        person(),
      ],
    );
  }
}
