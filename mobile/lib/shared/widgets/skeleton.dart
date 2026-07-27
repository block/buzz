import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

const _shimmerDuration = Duration(milliseconds: 2000);
const _skeletonOpacity = 0.1;
const _shimmerMinOpacity = 0.5;
const _revealDuration = Duration(milliseconds: 400);
const _revealBlur = 2.0;

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
    final colors = Theme.of(context).colorScheme;
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

class _SkeletonMask extends InheritedWidget {
  const _SkeletonMask({required super.child});

  static bool isActive(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_SkeletonMask>() != null;

  @override
  bool updateShouldNotify(_SkeletonMask oldWidget) => false;
}

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
                  child: ExcludeSemantics(excluding: loading, child: content),
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

/// A single rounded placeholder within a [SkeletonShimmer].
class SkeletonBar extends StatelessWidget {
  final double width;
  final double height;
  final BorderRadius? borderRadius;

  const SkeletonBar({
    required this.width,
    required this.height,
    this.borderRadius,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return ExcludeSemantics(
      child: SizedBox(
        width: width,
        height: height,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: _SkeletonMask.isActive(context)
                ? Colors.white
                : Theme.of(context).colorScheme.primary.withValues(alpha: 0.1),
            borderRadius: borderRadius ?? BorderRadius.circular(6),
          ),
        ),
      ),
    );
  }
}
