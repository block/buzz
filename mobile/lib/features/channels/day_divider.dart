import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// In-flow date label. The active date gains a glass capsule when it sticks.
class DayDivider extends StatelessWidget {
  final String label;
  final int? dayTimestamp;
  final ValueListenable<int?>? stickyDayTimestamp;

  const DayDivider({
    super.key,
    required this.label,
    this.dayTimestamp,
    this.stickyDayTimestamp,
  });

  Widget _buildOpacity(BuildContext context, {required bool isSticky}) {
    return ExcludeSemantics(
      excluding: isSticky,
      child: AnimatedOpacity(
        key: dayTimestamp == null
            ? null
            : ValueKey('channel-day-divider-opacity-$dayTimestamp'),
        duration: MediaQuery.disableAnimationsOf(context)
            ? Duration.zero
            : const Duration(milliseconds: 120),
        curve: Curves.easeOutCubic,
        opacity: isSticky ? 0 : 1,
        child: Text(
          label,
          style: context.textTheme.labelSmall?.copyWith(
            color: context.colors.onSurfaceVariant.withValues(alpha: 0.72),
            fontWeight: FontWeight.w500,
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final activeTimestamp = stickyDayTimestamp;
    final timestamp = dayTimestamp;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs + Grid.quarter),
      child: Center(
        child: activeTimestamp == null || timestamp == null
            ? _buildOpacity(context, isSticky: false)
            : ValueListenableBuilder<int?>(
                valueListenable: activeTimestamp,
                builder: (context, activeDayTimestamp, _) => _buildOpacity(
                  context,
                  isSticky: activeDayTimestamp == timestamp,
                ),
              ),
      ),
    );
  }
}
