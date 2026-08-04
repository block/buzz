part of 'skeleton.dart';

/// Stacks a skeleton and its content in the same slot, then reveals the content.
///
/// Entering the loading state is intentionally instant so reconnects do not
/// animate backwards. Leaving it cross-fades both layers while the skeleton
/// blurs out and the content sharpens over 400ms.
class SkeletonReveal extends HookWidget {
  final bool loading;
  final Widget skeleton;
  final Widget content;
  final bool shimmerEnabled;

  const SkeletonReveal({
    required this.loading,
    required this.skeleton,
    required this.content,
    this.shimmerEnabled = true,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final reveal = useAnimationController(
      duration: _revealDuration,
      initialValue: loading ? 0 : 1,
    );
    final previousLoading = usePrevious(loading);

    useEffect(() {
      if (reducedMotion || previousLoading == null) {
        reveal
          ..stop()
          ..value = loading ? 0 : 1;
      } else if (loading) {
        // Reset directly to the loading state. Only the reveal animates.
        reveal
          ..stop()
          ..value = 0;
      } else if (previousLoading) {
        reveal.forward(from: 0);
      }
      return null;
    }, [loading, reducedMotion]);

    return AnimatedBuilder(
      animation: reveal,
      builder: (context, _) {
        final progress = reveal.value;
        final skeletonBlur = reducedMotion ? 0.0 : _revealBlur * progress;
        final contentBlur = reducedMotion ? 0.0 : _revealBlur * (1 - progress);

        return Stack(
          fit: StackFit.expand,
          children: [
            Opacity(
              key: const Key('skeleton-reveal-content'),
              opacity: progress,
              child: ImageFiltered(
                enabled: contentBlur > 0.01,
                imageFilter: ImageFilter.blur(
                  sigmaX: contentBlur,
                  sigmaY: contentBlur,
                ),
                child: IgnorePointer(
                  ignoring: loading || progress < 1,
                  child: ExcludeFocus(
                    excluding: loading || progress < 1,
                    child: ExcludeSemantics(
                      excluding: loading || progress < 1,
                      child: content,
                    ),
                  ),
                ),
              ),
            ),
            Opacity(
              key: const Key('skeleton-reveal-placeholder'),
              opacity: 1 - progress,
              child: ImageFiltered(
                enabled: skeletonBlur > 0.01,
                imageFilter: ImageFilter.blur(
                  sigmaX: skeletonBlur,
                  sigmaY: skeletonBlur,
                ),
                child: IgnorePointer(
                  child: ExcludeSemantics(
                    excluding: !loading,
                    child: SkeletonShimmer(
                      enabled: loading && shimmerEnabled,
                      child: skeleton,
                    ),
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}
