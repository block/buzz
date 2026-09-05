part of '../thread_detail_page.dart';

const _landingHighlightDuration = Duration(seconds: 3);
const _landingHighlightDelay = Duration(milliseconds: 50);
const _landingHighlightTransitionDuration = Duration(milliseconds: 300);
const _landingHighlightOpacity = 0.12;

class _ThreadJumpToLatest extends StatelessWidget {
  const _ThreadJumpToLatest({
    required this.bottomInset,
    required this.visible,
    required this.onPressed,
  });

  final double bottomInset;
  final bool visible;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => Positioned(
    left: 0,
    right: 0,
    bottom: bottomInset + Grid.xs,
    child: Center(
      child: JumpToLatestSwitcher(
        id: 'thread',
        visible: visible,
        onPressed: onPressed,
      ),
    ),
  );
}

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
