import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// Responsive directory grid; larger accessibility text gets wider cards.
class AgentCardGrid extends StatelessWidget {
  const AgentCardGrid({
    required this.itemCount,
    required this.itemBuilder,
    this.cardHeight = 220,
    super.key,
  });
  final double cardHeight;
  final int itemCount;
  final IndexedWidgetBuilder itemBuilder;

  @override
  Widget build(BuildContext context) => SliverLayoutBuilder(
    builder: (context, constraints) {
      final scale = MediaQuery.textScalerOf(context).scale(16) / 16;
      final width = constraints.crossAxisExtent - Grid.xs * 2;
      final columns = ((width + Grid.twelve) / (160 * scale + Grid.twelve))
          .floor()
          .clamp(1, 4);
      return SliverPadding(
        padding: const EdgeInsets.fromLTRB(Grid.xs, Grid.xxs, Grid.xs, Grid.sm),
        sliver: SliverGrid.builder(
          itemCount: itemCount,
          gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
            crossAxisCount: columns,
            crossAxisSpacing: Grid.twelve,
            mainAxisSpacing: Grid.twelve,
            mainAxisExtent:
                cardHeight + (scale - 1).clamp(0, double.infinity) * 120,
          ),
          itemBuilder: itemBuilder,
        ),
      );
    },
  );
}
