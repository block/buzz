part of '../thread_detail_page.dart';

double _threadTailAlignmentForViewport({
  required double fullHeight,
  required double imeBottomInset,
  required bool usesFixedImeViewport,
  required double bottomInset,
}) {
  // iOS resizes the Scaffold body around the keyboard and removes that inset
  // from the body's MediaQuery. List alignment is relative to that smaller
  // viewport, so using fullHeight leaves the final reply behind the composer.
  final viewportHeight =
      (fullHeight - (usesFixedImeViewport ? 0 : imeBottomInset))
          .clamp(1.0, double.infinity)
          .toDouble();
  return (1 - bottomInset / viewportHeight).clamp(0.0, 1.0).toDouble();
}
