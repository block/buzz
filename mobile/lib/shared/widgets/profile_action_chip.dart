import 'dart:ui';

import 'profile_glass_surface.dart';

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
    this.sampled = false,
    this.useThreeButtonWidth = false,
    super.key,
  });

  final String label;
  final Widget? leading;
  final VoidCallback? onPressed;
  final bool interactive;
  final bool selected;
  final bool sampled;
  final bool useThreeButtonWidth;

  @override
  Widget build(BuildContext context) {
    final labelStyle = context.textTheme.labelMedium?.copyWith(
      fontWeight: FontWeight.w500,
      height: 20 / 14,
      letterSpacing: 0.035,
      color: sampled ? context.colors.onSurface : null,
    );
    final content = Row(
      mainAxisSize: useThreeButtonWidth ? MainAxisSize.max : MainAxisSize.min,
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        if (leading != null) ...[
          ExcludeSemantics(child: leading!),
          const SizedBox(width: Grid.half),
        ],
        Flexible(
          child: Text(
            label,
            style: labelStyle,
            textAlign: !sampled ? null : TextAlign.center,
            textHeightBehavior: !sampled
                ? null
                : const TextHeightBehavior(
                    leadingDistribution: TextLeadingDistribution.even,
                  ),
          ),
        ),
      ],
    );
    final dark = Theme.of(context).brightness == Brightness.dark;
    const padding = EdgeInsets.symmetric(
      horizontal: Grid.xs,
      vertical: Grid.twelve / 2,
    );
    final Widget capsule;
    if (sampled) {
      capsule = ProfileGlassSurface(
        shape: const StadiumBorder(),
        dark: dark,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 48),
          child: Padding(padding: padding, child: content),
        ),
      );
    } else {
      capsule = DecoratedBox(
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
                  border: useThreeButtonWidth
                      ? null
                      : Border(
                          top: BorderSide(
                            color: Colors.white.withValues(
                              alpha: dark ? 0.18 : 0.45,
                            ),
                          ),
                          bottom: BorderSide(
                            color: Colors.white.withValues(
                              alpha: dark ? 0.06 : 0.12,
                            ),
                          ),
                        ),
                ),
                child: Padding(padding: padding, child: content),
              ),
            ),
          ),
        ),
      );
    }
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
    final button = Semantics(
      selected: selected,
      child: TextButton(
        onPressed: onPressed,
        style: TextButton.styleFrom(
          padding: EdgeInsets.zero,
          minimumSize: const Size(48, 48),
          shape: const StadiumBorder(),
          foregroundColor: sampled
              ? lightColorScheme.onSurface
              : context.colors.onSurface,
        ),
        child: useThreeButtonWidth && !sampled
            ? DecoratedBox(
                position: DecorationPosition.foreground,
                decoration: ShapeDecoration(
                  shape: StadiumBorder(
                    side: BorderSide(
                      color: context.colors.inverseSurface.withValues(
                        alpha: .04,
                      ),
                      strokeAlign: BorderSide.strokeAlignOutside,
                    ),
                  ),
                ),
                child: capsule,
              )
            : capsule,
      ),
    );
    if (!useThreeButtonWidth) return button;
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = (constraints.maxWidth - 2 * Grid.xxs) / 3;
        return SizedBox(
          width: width.clamp(0.0, constraints.maxWidth),
          child: button,
        );
      },
    );
  }
}
