part of '../compose_bar.dart';

class _ComposerDockFrame extends StatelessWidget {
  final double widthFactor;
  final Widget child;

  const _ComposerDockFrame({required this.widthFactor, required this.child});

  @override
  Widget build(BuildContext context) {
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
            child: FractionallySizedBox(
              key: const ValueKey('composer-width-transition'),
              widthFactor: widthFactor,
              child: child,
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
                      child: surface == _AttachmentSurface.closed
                          ? _SuggestionPanelMotion(
                              duration: surfaceDuration,
                              alignment: Alignment.bottomLeft,
                              child: buildOverlayPanel(surface),
                            )
                          : buildOverlayPanel(surface),
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
