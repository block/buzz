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
        child: LayoutBuilder(
          builder: (context, constraints) => Row(
            children: [
              Expanded(
                child: Divider(
                  color: context.colors.outlineVariant,
                  thickness: 0.5,
                ),
              ),
              const SizedBox(width: Grid.xs),
              ConstrainedBox(
                constraints: BoxConstraints(
                  maxWidth: (constraints.maxWidth - Grid.xs * 2) * 0.6,
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (dayTimestamp != null) ...[
                      _CalendarDateTile(timestamp: dayTimestamp!),
                      const SizedBox(width: Grid.xxs),
                    ],
                    Flexible(
                      child: Text(
                        label,
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onSurfaceVariant.withValues(
                            alpha: 0.72,
                          ),
                          fontWeight: FontWeight.w500,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: Grid.xs),
              Expanded(
                child: Divider(
                  color: context.colors.outlineVariant,
                  thickness: 0.5,
                ),
              ),
            ],
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
      padding: const EdgeInsets.symmetric(vertical: Grid.gutter),
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

/// Decorative desktop calendar artwork; the adjacent label owns semantics.
class _CalendarDateTile extends StatelessWidget {
  const _CalendarDateTile({required this.timestamp});

  final int timestamp;

  @override
  Widget build(BuildContext context) {
    final day = DateTime.fromMillisecondsSinceEpoch(timestamp * 1000).day;
    return ExcludeSemantics(
      child: Container(
        key: ValueKey('day-calendar-$timestamp'),
        width: Grid.gutter,
        height: Grid.gutter,
        clipBehavior: Clip.antiAlias,
        decoration: BoxDecoration(
          color: Colors.white,
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(color: context.colors.outlineVariant, width: 0.5),
        ),
        child: Column(
          children: [
            const SizedBox(
              height: Grid.gutter * 7 / 24,
              width: double.infinity,
              child: ColoredBox(color: Color(0xFFFF0000)),
            ),
            Expanded(
              child: Center(
                child: Text(
                  '$day',
                  textScaler: TextScaler.noScaling,
                  style: const TextStyle(
                    color: Color(0xFF111111),
                    fontSize: Grid.gutter / 2,
                    height: 1,
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
