part of '../compose_bar.dart';

class _ComposerDockFrame extends HookWidget {
  final Animation<double> expansionAnimation;
  final bool forceFullWidth;
  final Widget child;

  const _ComposerDockFrame({
    required this.expansionAnimation,
    required this.forceFullWidth,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final recordingTransition = useAnimationController(
      duration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 200),
      reverseDuration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 140),
      initialValue: forceFullWidth ? 1 : 0,
    );
    useEffect(() {
      if (forceFullWidth) {
        recordingTransition.value = math.max(
          recordingTransition.value,
          expansionAnimation.value.clamp(0.0, 1.0),
        );
        recordingTransition.forward();
      } else {
        recordingTransition.reverse();
      }
      return null;
    }, [forceFullWidth, reducedMotion]);
    const compactVerticalOffset = Grid.twelve + Grid.quarter;
    final visibleBottomGutter = math.max(
      Grid.twelve,
      MediaQuery.viewPaddingOf(context).bottom +
          Grid.xxs -
          compactVerticalOffset,
    );
    final recordingGutterDelta = visibleBottomGutter - Grid.twelve;
    final backdropHeight = mobileTabFooterBackdropHeight(context);
    return Stack(
      clipBehavior: Clip.none,
      children: [
        Positioned(
          key: const ValueKey('composer-footer-gradient'),
          left: 0,
          right: 0,
          bottom: 0,
          height: backdropHeight,
          child: IgnorePointer(
            child: MobileTabFooterBackdrop(height: backdropHeight),
          ),
        ),
        Padding(
          padding: EdgeInsets.only(
            left: Grid.twelve,
            right: Grid.twelve,
            bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.xxs,
          ),
          child: Align(
            alignment: Alignment.bottomCenter,
            child: AnimatedBuilder(
              animation: Listenable.merge([
                expansionAnimation,
                recordingTransition,
              ]),
              child: child,
              builder: (context, child) {
                final widthProgress = math.max(
                  expansionAnimation.value.clamp(0.0, 1.0),
                  recordingTransition.value,
                );
                final expansionProgress = expansionAnimation.value.clamp(
                  0.0,
                  1.0,
                );
                final compactPositionProgress = 1 - expansionProgress;
                return Transform.translate(
                  key: const ValueKey('composer-position-transition'),
                  offset: Offset(
                    0,
                    compactVerticalOffset * compactPositionProgress,
                  ),
                  child: Padding(
                    key: const ValueKey('composer-recording-outer-gutter'),
                    padding: EdgeInsets.symmetric(
                      horizontal: forceFullWidth
                          ? recordingGutterDelta * compactPositionProgress
                          : 0,
                    ),
                    child: FractionallySizedBox(
                      key: const ValueKey('composer-width-transition'),
                      widthFactor: 0.85 + 0.15 * widthProgress,
                      child: child,
                    ),
                  ),
                );
              },
            ),
          ),
        ),
      ],
    );
  }
}

class _ComposerOverlayPortal extends StatelessWidget {
  final OverlayPortalController controller;
  final ValueListenable<_AttachmentSurface> attachmentSurface;
  final bool reducedMotion;
  final Widget Function(_AttachmentSurface surface) buildOverlayPanel;
  final VoidCallback onDismissAttachmentSurface;
  final Widget child;

  const _ComposerOverlayPortal({
    required this.controller,
    required this.attachmentSurface,
    required this.reducedMotion,
    required this.buildOverlayPanel,
    required this.onDismissAttachmentSurface,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    return OverlayPortal.overlayChildLayoutBuilder(
      controller: controller,
      overlayChildBuilder: (context, layoutInfo) {
        final composerOrigin = MatrixUtils.transformPoint(
          layoutInfo.childPaintTransform,
          Offset.zero,
        );
        return ValueListenableBuilder<_AttachmentSurface>(
          valueListenable: attachmentSurface,
          builder: (context, surface, _) {
            final surfaceDuration = reducedMotion
                ? Duration.zero
                : Duration(
                    milliseconds:
                        surface == _AttachmentSurface.camera ||
                            surface == _AttachmentSurface.photos
                        ? 320
                        : 250,
                  );
            final expandedSurfaceCoversComposer =
                surface == _AttachmentSurface.camera ||
                surface == _AttachmentSurface.photos;
            final surfaceLeft = expandedSurfaceCoversComposer
                ? Grid.twelve
                : composerOrigin.dx;
            final surfaceWidth = expandedSurfaceCoversComposer
                ? layoutInfo.overlaySize.width - (Grid.twelve * 2)
                : layoutInfo.childSize.width;
            final overlayAnchorY =
                composerOrigin.dy +
                (expandedSurfaceCoversComposer
                    ? layoutInfo.childSize.height + Grid.twelve
                    : 0);
            return Stack(
              children: [
                if (surface != _AttachmentSurface.closed)
                  Positioned(
                    left: 0,
                    top: 0,
                    right: 0,
                    height: composerOrigin.dy,
                    child: ExcludeSemantics(
                      child: GestureDetector(
                        key: const ValueKey('attachment-dismiss-barrier'),
                        behavior: HitTestBehavior.opaque,
                        onTap: onDismissAttachmentSurface,
                      ),
                    ),
                  ),
                AnimatedPositioned(
                  duration: surfaceDuration,
                  curve:
                      surface == _AttachmentSurface.camera ||
                          surface == _AttachmentSurface.photos
                      ? const Cubic(0.34, 1.25, 0.64, 1)
                      : const Cubic(0.22, 1, 0.36, 1),
                  left: surfaceLeft,
                  bottom: layoutInfo.overlaySize.height - overlayAnchorY,
                  width: surfaceWidth,
                  child: ClipRect(
                    child: Padding(
                      padding: const EdgeInsets.only(bottom: Grid.xxs),
                      child: buildOverlayPanel(surface),
                    ),
                  ),
                ),
              ],
            );
          },
        );
      },
      child: child,
    );
  }
}
