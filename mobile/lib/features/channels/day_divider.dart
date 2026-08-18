import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// Desktop-parity day separator with a centered label over a horizontal rule.
class DayDivider extends StatelessWidget {
  final String label;

  const DayDivider({super.key, required this.label});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
      child: SizedBox(
        width: double.infinity,
        child: Stack(
          alignment: Alignment.center,
          children: [
            Positioned(
              left: 0,
              right: 0,
              child: Divider(
                height: 1,
                thickness: 1,
                color: context.colors.outlineVariant.withValues(alpha: 0.35),
              ),
            ),
            Container(
              padding: const EdgeInsets.symmetric(
                horizontal: Grid.xxs + Grid.quarter,
                vertical: Grid.half,
              ),
              decoration: BoxDecoration(
                color: context.colors.surface,
                borderRadius: BorderRadius.circular(Radii.dialog),
                border: Border.all(
                  color: context.colors.outlineVariant.withValues(alpha: 0.7),
                ),
              ),
              child: Text(
                label,
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant.withValues(alpha: 0.7),
                  letterSpacing: 0.22,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
