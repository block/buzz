import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';

/// Frosted pill that jumps a message timeline back to its newest entry.
///
/// Shared by the channel timeline and the thread detail page. Callers show
/// it only while the newest message is scrolled out of view and hide it once
/// the tail is visible again.
class JumpToLatestButton extends StatelessWidget {
  final VoidCallback onPressed;

  /// Optional key for the frosted surface container, kept separate from the
  /// widget [key] so tests can target the surface decoration directly.
  final Key? surfaceKey;

  const JumpToLatestButton({
    required this.onPressed,
    this.surfaceKey,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final borderRadius = BorderRadius.circular(Radii.full);
    return Semantics(
      button: true,
      child: ClipRRect(
        borderRadius: borderRadius,
        child: BackdropFilter(
          filter: ImageFilter.blur(sigmaX: 20, sigmaY: 20),
          child: Container(
            key: surfaceKey,
            decoration: BoxDecoration(
              color: context.colors.surface.withValues(alpha: 0.5),
              borderRadius: borderRadius,
              border: Border.all(
                color: Colors.black.withValues(alpha: 0.04),
                width: 1,
              ),
            ),
            child: Material(
              type: MaterialType.transparency,
              child: InkWell(
                onTap: onPressed,
                borderRadius: borderRadius,
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.gutter,
                    vertical: Grid.xxs,
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        LucideIcons.arrowDown,
                        size: 16,
                        color: context.colors.onSurface,
                      ),
                      const SizedBox(width: Grid.half),
                      Text(
                        'Latest',
                        style: context.textTheme.labelLarge?.copyWith(
                          color: context.colors.onSurface,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
