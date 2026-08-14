import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';

/// Shared channel/thread control for returning to the newest message.
class LatestMessageButton extends StatelessWidget {
  /// Returns the message list to its newest item.
  final VoidCallback onPressed;

  /// Optional key for inspecting or measuring the decorated glass surface.
  final Key? surfaceKey;

  /// Creates the shared control for returning to the newest message.
  const LatestMessageButton({
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
