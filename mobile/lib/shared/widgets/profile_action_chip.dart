import 'dart:ui';

import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Compact desktop-style profile capsule with a mobile-sized touch target.
class ProfileActionChip extends StatelessWidget {
  const ProfileActionChip({
    required this.label,
    this.leading,
    this.onPressed,
    this.interactive = true,
    this.selected = false,
    super.key,
  });

  final String label;
  final Widget? leading;
  final VoidCallback? onPressed;
  final bool interactive;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final content = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (leading != null) ...[
          ExcludeSemantics(child: leading!),
          const SizedBox(width: Grid.half),
        ],
        Text(
          label,
          style: context.textTheme.labelMedium?.copyWith(
            fontWeight: FontWeight.w500,
            height: 20 / 14,
            letterSpacing: 0.035,
          ),
        ),
      ],
    );
    final dark = Theme.of(context).brightness == Brightness.dark;
    final capsule = DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(Radii.full),
        boxShadow: const [
          BoxShadow(color: Color(0x1A000000), blurRadius: 2),
          BoxShadow(
            color: Color(0x0F000000),
            offset: Offset(0, 4),
            blurRadius: 8,
            spreadRadius: -2,
          ),
        ],
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(Radii.full),
        child: BackdropFilter(
          filter: ImageFilter.blur(sigmaX: 16, sigmaY: 16),
          child: Material(
            color: dark ? const Color(0xC71C1C1C) : const Color(0xC7FFFFFF),
            shape: const StadiumBorder(),
            child: DecoratedBox(
              decoration: BoxDecoration(
                border: Border(
                  top: BorderSide(
                    color: Colors.white.withValues(alpha: dark ? 0.18 : 0.45),
                  ),
                  bottom: BorderSide(
                    color: Colors.white.withValues(alpha: dark ? 0.06 : 0.12),
                  ),
                ),
              ),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.xs,
                  vertical: Grid.twelve / 2,
                ),
                child: content,
              ),
            ),
          ),
        ),
      ),
    );
    if (!interactive) {
      return Semantics(
        label: label,
        excludeSemantics: true,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 48),
          child: Center(widthFactor: 1, child: capsule),
        ),
      );
    }
    return Semantics(
      selected: selected,
      child: TextButton(
        onPressed: onPressed,
        style: TextButton.styleFrom(
          padding: EdgeInsets.zero,
          minimumSize: const Size(48, 48),
          shape: const StadiumBorder(),
          foregroundColor: context.colors.onSurface,
        ),
        child: capsule,
      ),
    );
  }
}
