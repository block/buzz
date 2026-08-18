import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/theme.dart';
import 'buzz_loading_indicator.dart';

/// Equal-width icon action used by profile and profile-adjacent surfaces.
class BuzzActionTile extends StatelessWidget {
  /// Creates an action tile with an optional loading state.
  const BuzzActionTile({
    super.key,
    required this.icon,
    required this.label,
    required this.onTap,
    this.isEnabled = true,
    this.isLoading = false,
    this.loadingSemanticLabel,
  });

  /// Icon shown when the tile is not loading.
  final IconData? icon;

  /// Label shown below the icon.
  final String label;

  /// Called when the tile is tapped.
  final VoidCallback onTap;

  /// Whether the tile accepts taps.
  final bool isEnabled;

  /// Whether to show a loading indicator instead of [icon].
  final bool isLoading;

  /// Accessibility label for the loading indicator.
  final String? loadingSemanticLabel;

  @override
  Widget build(BuildContext context) {
    final canTap = isEnabled && !isLoading;
    return GestureDetector(
      onTap: canTap
          ? () {
              unawaited(HapticFeedback.lightImpact());
              onTap();
            }
          : null,
      behavior: HitTestBehavior.opaque,
      child: Opacity(
        opacity: isEnabled ? 1 : 0.5,
        child: Container(
          width: double.infinity,
          height: 68 + (Grid.xxs * 2),
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(Radii.dialog),
          ),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              if (isLoading)
                BuzzLoadingIndicator(
                  size: 22,
                  color: context.colors.onSurface,
                  semanticLabel: loadingSemanticLabel ?? label,
                )
              else
                Icon(icon, size: 22, color: context.colors.onSurface),
              const SizedBox(height: Grid.xxs),
              Text(
                label,
                style: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.onSurface,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
