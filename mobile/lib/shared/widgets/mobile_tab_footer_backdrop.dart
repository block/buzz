import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Height of the floating mobile tab bar, excluding its bottom clearance.
const mobileTabBarHeight = 56.0;

/// Gap between the floating mobile tab bar and the bottom safe area.
const mobileTabBarBottomGap = Grid.twelve;

/// Returns the shared footer backdrop height, including the logical safe area.
double mobileTabFooterBackdropHeight(BuildContext context) =>
    mobileTabBarHeight +
    mobileTabBarBottomGap +
    MediaQuery.paddingOf(context).bottom +
    Grid.xl +
    Grid.gutter;

/// Builds the shared transparent-to-surface footer fade.
///
/// Kept separate from [MobileTabFooterBackdrop] so floating controls such as
/// the channel composer can paint the exact same fade behind their own content.
LinearGradient mobileTabFooterBackdropGradient(
  BuildContext context, {
  List<double> stops = const [0, 0.5, 1],
  List<double> opacities = const [0, 0.75, 1],
}) {
  assert(stops.length == opacities.length);
  final surface = context.colors.surface;
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

  /// Creates a footer backdrop with the required [height].
  ///
  /// Override [stops] and [opacities] together to customize the gradient.
  const MobileTabFooterBackdrop({
    super.key,
    required this.height,
    this.stops = const [0, 0.5, 1],
    this.opacities = const [0, 0.75, 1],
  }) : assert(stops.length == opacities.length);

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
          ),
        ),
      ),
    );
  }
}
