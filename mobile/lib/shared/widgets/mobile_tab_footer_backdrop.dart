import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Height of the floating mobile tab bar, excluding its bottom clearance.
const mobileTabBarHeight = 56.0;

/// Gap between the floating mobile tab bar and the bottom safe area.
const mobileTabBarBottomGap = Grid.twelve;

/// Fixed visual height of the shared footer fade behind the floating tab bar.
double mobileTabFooterBackdropHeight(BuildContext _) => 180;

/// Builds the shared transparent-to-surface footer fade.
///
/// Kept separate from [MobileTabFooterBackdrop] so floating controls such as
/// the channel composer can paint the exact same fade behind their own content.
LinearGradient mobileTabFooterBackdropGradient(
  BuildContext context, {
  List<double> stops = const [0, 0.18, 0.38, 0.6, 0.8, 1],
  List<double> opacities = const [0, 0.03, 0.12, 0.34, 0.7, 1],
  Color? tint,
  double tintBlend = 0,
}) {
  assert(stops.length == opacities.length);
  assert(tintBlend >= 0 && tintBlend <= 1);
  final surface = Color.lerp(context.colors.surface, tint, tintBlend)!;
  return LinearGradient(
    begin: Alignment.topCenter,
    end: Alignment.bottomCenter,
    stops: stops,
    colors: [
      for (final opacity in opacities) surface.withValues(alpha: opacity),
    ],
  );
}

/// Shared fade behind the floating mobile tab bar.
class MobileTabFooterBackdrop extends StatelessWidget {
  /// Vertical extent of the backdrop in logical pixels.
  final double height;

  /// Gradient stop positions, from the transparent top to the opaque bottom.
  final List<double> stops;

  /// Surface-color alpha values paired with [stops].
  final List<double> opacities;

  /// Optional color blended into the surface before opacity is applied.
  final Color? tint;

  /// Amount of [tint] mixed into the surface, from 0 to 1.
  final double tintBlend;

  /// Creates a footer backdrop with the required [height].
  ///
  /// Override [stops] and [opacities] together to customize the gradient.
  const MobileTabFooterBackdrop({
    super.key,
    required this.height,
    this.stops = const [0, 0.18, 0.38, 0.6, 0.8, 1],
    this.opacities = const [0, 0.03, 0.12, 0.34, 0.7, 1],
    this.tint,
    this.tintBlend = 0,
  }) : assert(stops.length == opacities.length),
       assert(tintBlend >= 0 && tintBlend <= 1);

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: height,
      width: double.infinity,
      child: DecoratedBox(
        decoration: BoxDecoration(
          gradient: mobileTabFooterBackdropGradient(
            context,
            stops: stops,
            opacities: opacities,
            tint: tint,
            tintBlend: tintBlend,
          ),
        ),
      ),
    );
  }
}
