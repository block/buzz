import 'package:flutter/material.dart';

/// A fixed-size conversation identity with up to four distinct portraits.
class DmHeaderCluster extends StatelessWidget {
  const DmHeaderCluster({
    super.key,
    required this.count,
    required this.builder,
  });

  final int count;
  final Widget Function(int index, double size) builder;

  @override
  Widget build(BuildContext context) {
    if (count == 0) return const SizedBox.shrink();
    if (count == 1) {
      return SizedBox.square(dimension: 64, child: builder(0, 64));
    }
    // Centers and diameters are fractions of the 60pt inner circle.
    final positions = switch (count) {
      2 => [(0.355, 0.355, 0.58), (0.709, 0.709, 0.38)],
      3 => [(0.360, 0.340, 0.50), (0.380, 0.780, 0.34), (0.736, 0.622, 0.40)],
      _ => [
        (0.350, 0.340, 0.50),
        (0.745, 0.340, 0.25),
        (0.350, 0.760, 0.30),
        (0.706, 0.678, 0.39),
      ],
    };
    final theme = Theme.of(context);
    final inverse = theme.brightness == Brightness.dark
        ? Colors.white
        : Colors.black;
    return DecoratedBox(
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        border: Border.all(
          color: inverse.withValues(alpha: 0.07),
          width: 1,
          strokeAlign: BorderSide.strokeAlignOutside,
        ),
      ),
      child: ClipOval(
        child: ColoredBox(
          color: theme.scaffoldBackgroundColor,
          child: SizedBox.square(
            dimension: 64,
            child: Stack(
              children: [
                for (var i = 0; i < positions.length; i++)
                  Positioned(
                    left: 2 + 60 * (positions[i].$1 - positions[i].$3 / 2),
                    top: 2 + 60 * (positions[i].$2 - positions[i].$3 / 2),
                    child: builder(i, 60 * positions[i].$3),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
