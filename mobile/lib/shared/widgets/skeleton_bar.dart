part of 'skeleton.dart';

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
                : context.colors.primary.withValues(alpha: _skeletonOpacity),
            borderRadius: borderRadius ?? BorderRadius.circular(Radii.sm),
          ),
        ),
      ),
    );
  }
}
