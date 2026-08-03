import 'package:flutter/material.dart';

/// Keeps image-viewer shared-element motion consistent at every source.
class MediaViewerHero extends StatelessWidget {
  /// The identity shared by the inline image and full-screen image.
  final Object tag;

  /// The image rendered during and after the shared-element transition.
  final Widget child;

  /// Creates an image-viewer shared element.
  const MediaViewerHero({super.key, required this.tag, required this.child});

  @override
  Widget build(BuildContext context) {
    return Hero(
      tag: tag,
      createRectTween: (begin, end) => RectTween(begin: begin, end: end),
      flightShuttleBuilder:
          (
            flightContext,
            animation,
            flightDirection,
            fromHeroContext,
            toHeroContext,
          ) {
            final sourceHero = fromHeroContext.widget;
            final destinationHero = toHeroContext.widget;
            final sourceChild = sourceHero is Hero ? sourceHero.child : child;
            final destinationChild = destinationHero is Hero
                ? destinationHero.child
                : child;
            return _MediaViewerHeroFlight(
              animation: animation,
              sourceChild: sourceChild,
              destinationChild: destinationChild,
            );
          },
      child: child,
    );
  }
}

class _MediaViewerHeroFlight extends StatelessWidget {
  final Animation<double> animation;
  final Widget sourceChild;
  final Widget destinationChild;

  const _MediaViewerHeroFlight({
    required this.animation,
    required this.sourceChild,
    required this.destinationChild,
  });

  @override
  Widget build(BuildContext context) {
    final destinationOpacity = CurvedAnimation(
      parent: animation,
      curve: const Interval(0.18, 0.82, curve: Curves.easeInOutCubic),
    );
    return Stack(
      fit: StackFit.expand,
      children: [
        FadeTransition(
          opacity: ReverseAnimation(destinationOpacity),
          child: sourceChild,
        ),
        FadeTransition(opacity: destinationOpacity, child: destinationChild),
      ],
    );
  }
}
