import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';

/// Space between avatar-editor controls and their labels.
const avatarEditorOptionLabelGap = Grid.half;

/// A labelled circular option used by the profile avatar editor rails.
class AvatarEditorOptionButton extends StatelessWidget {
  /// Creates a selectable avatar-editor rail option.
  const AvatarEditorOptionButton({
    super.key,
    required this.icon,
    this.iosIcon,
    required this.label,
    required this.selected,
    required this.onTap,
    this.labelMaxWidth,
  });

  /// The symbol displayed inside the circular control.
  final IconData icon;

  /// Native symbol used by the iOS liquid-glass control.
  final IosGlassNavigationIcon? iosIcon;

  /// The text displayed beneath the control.
  final String label;

  /// Whether this option represents the active editor section.
  final bool selected;

  /// Called when the option is selected, or null when disabled.
  final VoidCallback? onTap;

  /// Optional width constraint for the label's overflow region.
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
      final width = labelMaxWidth;
      if (width != null) {
        return SizedBox(
          height: 20,
          child: OverflowBox(
            maxWidth: width,
            maxHeight: 20,
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: context.textTheme.labelSmall?.copyWith(
                color: selected
                    ? context.colors.onSurface
                    : context.colors.onSurfaceVariant,
              ),
            ),
          ),
        );
      }
      return Text(
        label,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: context.textTheme.labelSmall?.copyWith(
          color: selected
              ? context.colors.onSurface
              : context.colors.onSurfaceVariant,
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
          const SizedBox(height: avatarEditorOptionLabelGap),
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
                const SizedBox(height: avatarEditorOptionLabelGap),
                labelWidget(),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
