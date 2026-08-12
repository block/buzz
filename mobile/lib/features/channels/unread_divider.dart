import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// Desktop-parity "New" separator marking the first unread message.
///
/// Same construction as [DayDivider] — a centered label over a horizontal
/// rule — but tinted with the error colour so it reads as a position marker
/// rather than a date heading.
class UnreadDivider extends StatelessWidget {
  const UnreadDivider({super.key});

  @override
  Widget build(BuildContext context) {
    final accent = context.colors.error;
    return Semantics(
      label: 'New messages',
      child: Padding(
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
                  color: accent.withValues(alpha: 0.45),
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
                  border: Border.all(color: accent.withValues(alpha: 0.7)),
                ),
                child: Text(
                  'New',
                  style: context.textTheme.labelSmall?.copyWith(
                    color: accent,
                    letterSpacing: 0.22,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
