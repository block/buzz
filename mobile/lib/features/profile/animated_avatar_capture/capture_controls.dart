part of '../animated_avatar_capture.dart';

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
          child: Material(
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
                          key: const ValueKey('animated-avatar-capturing'),
                          size: 24,
                          color: context.colors.surface,
                          semanticLabel: 'Capturing animated avatar',
                        )
                      : Text(
                          'Record',
                          key: const ValueKey('animated-avatar-record-label'),
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
    final orientation = controller.value.deviceOrientation;
    final isLandscape =
        orientation == DeviceOrientation.landscapeLeft ||
        orientation == DeviceOrientation.landscapeRight;
    final aspectRatio = isLandscape
        ? controller.value.aspectRatio
        : 1 / controller.value.aspectRatio;
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
