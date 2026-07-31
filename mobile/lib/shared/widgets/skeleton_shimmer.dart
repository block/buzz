part of 'skeleton.dart';

/// Sweeps one shared highlight across a group of skeleton elements.
///
/// This mirrors desktop's two-second linear shimmer. Reduced-motion users see
/// the same element shapes without the moving highlight.
class SkeletonShimmer extends HookWidget {
  final Widget child;
  final bool enabled;

  const SkeletonShimmer({required this.child, this.enabled = true, super.key});

  @override
  Widget build(BuildContext context) {
    final animation = useAnimationController(duration: _shimmerDuration);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final colors = context.colors;
    final baseColor = colors.primary.withValues(alpha: _skeletonOpacity);
    final highlightColor = colors.primary.withValues(
      alpha: _skeletonOpacity * _shimmerMinOpacity,
    );

    useEffect(() {
      if (reducedMotion || !enabled) {
        animation
          ..stop()
          ..value = 0;
      } else {
        animation.repeat();
      }
      return animation.stop;
    }, [animation, enabled, reducedMotion]);

    if (reducedMotion || !enabled) {
      return _SkeletonMask(
        child: ColorFiltered(
          colorFilter: ColorFilter.mode(baseColor, BlendMode.srcIn),
          child: child,
        ),
      );
    }

    return _SkeletonMask(
      child: RepaintBoundary(
        child: AnimatedBuilder(
          animation: animation,
          child: child,
          builder: (context, child) {
            final center = -1.5 + (animation.value * 3);
            return ShaderMask(
              blendMode: BlendMode.srcIn,
              shaderCallback: (bounds) => LinearGradient(
                begin: Alignment(center - 1, 0),
                end: Alignment(center + 1, 0),
                colors: [
                  baseColor,
                  baseColor,
                  highlightColor,
                  baseColor,
                  baseColor,
                ],
                stops: const [0, 0.34, 0.5, 0.66, 1],
              ).createShader(bounds),
              child: child,
            );
          },
        ),
      ),
    );
  }
}
