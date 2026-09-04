import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/theme.dart';
import 'ios_glass_navigation_button.dart';

/// A labelled circular mode option matching the profile editor controls.
class EditorOptionButton extends StatelessWidget {
  const EditorOptionButton({
    super.key,
    required this.icon,
    this.iosIcon,
    required this.label,
    required this.selected,
    required this.onTap,
    this.labelMaxWidth,
  });

  final IconData icon;
  final IosGlassNavigationIcon? iosIcon;
  final String label;
  final bool selected;
  final VoidCallback? onTap;
  final double? labelMaxWidth;

  @override
  Widget build(BuildContext context) {
    final handleTap = onTap == null
        ? null
        : () {
            unawaited(HapticFeedback.selectionClick());
            onTap!();
          };
    Widget labelWidget() {
      final style = context.textTheme.labelSmall?.copyWith(
        color: selected
            ? context.colors.onSurface
            : context.colors.onSurfaceVariant,
      );
      final width = labelMaxWidth;
      if (width == null) {
        return Text(
          label,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: style,
        );
      }
      return SizedBox(
        height: 20,
        child: OverflowBox(
          maxWidth: width,
          maxHeight: 20,
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: style,
          ),
        ),
      );
    }

    if (defaultTargetPlatform == TargetPlatform.iOS && iosIcon != null) {
      return Column(
        children: [
          IosGlassNavigationButton(
            icon: iosIcon!,
            semanticLabel: label,
            onPressed: handleTap,
            width: 64,
            height: 64,
            controlSize: 64,
            foregroundColor: selected
                ? context.colors.primary
                : context.colors.onSurface,
            isSelected: selected,
          ),
          const SizedBox(height: Grid.half),
          ExcludeSemantics(child: labelWidget()),
        ],
      );
    }
    return Semantics(
      label: label,
      button: true,
      selected: selected,
      enabled: handleTap != null,
      onTap: handleTap,
      child: ExcludeSemantics(
        child: InkResponse(
          radius: 34,
          onTap: handleTap,
          child: SizedBox(
            width: double.infinity,
            child: Column(
              children: [
                AnimatedContainer(
                  duration: MediaQuery.disableAnimationsOf(context)
                      ? Duration.zero
                      : const Duration(milliseconds: 150),
                  curve: Curves.easeOutCubic,
                  width: 64,
                  height: 64,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: selected
                        ? context.colors.onSurface
                        : context.colors.surfaceContainerHighest,
                  ),
                  child: Icon(
                    icon,
                    size: 26,
                    color: selected
                        ? context.colors.surface
                        : context.colors.onSurface,
                  ),
                ),
                const SizedBox(height: Grid.half),
                labelWidget(),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
