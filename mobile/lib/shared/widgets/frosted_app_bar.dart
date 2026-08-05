import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';
import 'directional_transition_scope.dart';

/// Minimum height of the frosted app bar content area below the safe area.
const _kBarContentMinHeight = Grid.xxs + 32 + Grid.xxs; // 48
const _kBottomBorderWidth = 1.0;

TextStyle _effectiveTitleStyle(BuildContext context, TextStyle? titleStyle) {
  final baseStyle =
      context.textTheme.titleMedium ??
      const TextStyle(fontSize: 20, height: 1.3);
  return baseStyle.copyWith(fontWeight: FontWeight.w600).merge(titleStyle);
}

double _barContentHeight(
  BuildContext context,
  TextStyle? titleStyle,
  double titleContentHeight,
) {
  final style = _effectiveTitleStyle(context, titleStyle);
  final scaledFontSize = MediaQuery.textScalerOf(
    context,
  ).scale(style.fontSize ?? 20);
  final scaledTitleHeight = scaledFontSize * (style.height ?? 1);
  final effectiveTitleHeight = titleContentHeight > scaledTitleHeight
      ? titleContentHeight
      : scaledTitleHeight;
  final accessibleHeight = Grid.xxs + effectiveTitleHeight + Grid.xxs;
  return accessibleHeight > _kBarContentMinHeight
      ? accessibleHeight
      : _kBarContentMinHeight;
}

/// Returns the total height of the [FrostedAppBar] including safe area padding.
///
/// Use this to add top spacing to body content so it starts below the bar.
/// Pass the same [titleStyle] and [titleContentHeight] to the bar and this
/// helper when customizing them.
double frostedAppBarHeight(
  BuildContext context, {
  double bottomHeight = 0,
  TextStyle? titleStyle,
  double titleContentHeight = 0,
}) {
  return MediaQuery.paddingOf(context).top +
      _barContentHeight(context, titleStyle, titleContentHeight) +
      bottomHeight +
      _kBottomBorderWidth;
}

/// A frosted-glass floating app bar designed to sit inside a [Stack].
///
/// Renders as a [Positioned] widget pinned to the top of its parent Stack.
/// Content scrolls underneath with a translucent backdrop blur effect.
class FrostedAppBar extends StatelessWidget {
  /// Widget displayed on the leading (left) side. If null and the navigator
  /// can pop, a back button is shown automatically.
  final Widget? leading;

  /// Whether to infer a back button from the current navigator.
  final bool automaticallyImplyLeading;

  /// Widget displayed in the center/title area.
  final Widget? title;

  /// Optional style merged over the default title style.
  final TextStyle? titleStyle;

  /// Scaled height needed by a custom title with multiple text lines.
  ///
  /// Pass the same value to [frostedAppBarHeight] when spacing body content.
  final double titleContentHeight;

  /// Optional content displayed below the title row in the same surface.
  final Widget? bottom;

  /// Height reserved for [bottom].
  final double bottomHeight;

  /// Widgets displayed on the trailing (right) side.
  final List<Widget> actions;

  /// Horizontal inset for the app bar's leading, title, and actions.
  final double horizontalInset;

  /// Color applied to icons in the app bar.
  final Color? iconColor;

  /// Paints over the frosted fill instead of the default translucent surface.
  /// Used by the Buzz themes to carry their branded gradient across the app's
  /// top section — see [buzzTopSectionGradient].
  final Gradient? gradient;

  const FrostedAppBar({
    super.key,
    this.leading,
    this.automaticallyImplyLeading = true,
    this.title,
    this.titleStyle,
    this.titleContentHeight = 0,
    this.bottom,
    this.bottomHeight = 0,
    this.actions = const [],
    this.horizontalInset = Grid.quarter,
    this.iconColor,
    this.gradient,
  }) : assert(bottom == null || bottomHeight > 0);

  @override
  Widget build(BuildContext context) {
    final topPadding = MediaQuery.paddingOf(context).top;
    final canPop = Navigator.canPop(context);
    final effectiveTitleStyle = _effectiveTitleStyle(context, titleStyle);
    final barContentHeight = _barContentHeight(
      context,
      titleStyle,
      titleContentHeight,
    );

    final effectiveLeading =
        leading ??
        (automaticallyImplyLeading && canPop
            ? SizedBox(
                width: 48,
                height: 48,
                child: IconButton(
                  onPressed: () => Navigator.of(context).pop(),
                  color: iconColor,
                  icon: const Icon(LucideIcons.chevronLeft),
                  tooltip: 'Back',
                ),
              )
            : null);

    return Positioned(
      top: 0,
      left: 0,
      right: 0,
      child: ClipRect(
        child: BackdropFilter(
          filter: ImageFilter.blur(sigmaX: 20, sigmaY: 20),
          child: Container(
            key: const ValueKey('frosted-app-bar-background'),
            padding: EdgeInsets.only(top: topPadding),
            decoration: BoxDecoration(
              // A gradient and a color cannot both paint, so the gradient
              // replaces the frosted surface fill when one is supplied.
              color: gradient == null
                  ? context.colors.surface.withValues(alpha: 0.5)
                  : null,
              gradient: gradient,
              border: Border(
                bottom: BorderSide(
                  color: context.colors.outlineVariant.withValues(alpha: 0.3),
                  width: _kBottomBorderWidth,
                ),
              ),
            ),
            child: DirectionalTransitionMotion(
              transformKey: const ValueKey(
                'frosted-app-bar-content-transition-transform',
              ),
              opacityKey: const ValueKey(
                'frosted-app-bar-content-transition-opacity',
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  SizedBox(
                    height: barContentHeight,
                    child: Padding(
                      padding: EdgeInsets.symmetric(
                        horizontal: horizontalInset,
                      ),
                      child: IconTheme.merge(
                        data: IconThemeData(color: iconColor),
                        child: Row(
                          children: [
                            ?effectiveLeading,
                            if (title != null)
                              Expanded(
                                child: Padding(
                                  padding: EdgeInsets.only(
                                    left: effectiveLeading != null
                                        ? 0
                                        : Grid.gutter - Grid.quarter,
                                    right: actions.isEmpty
                                        ? Grid.gutter - Grid.quarter
                                        : 0,
                                  ),
                                  child: DefaultTextStyle.merge(
                                    style: effectiveTitleStyle,
                                    overflow: TextOverflow.ellipsis,
                                    maxLines: 1,
                                    child: title!,
                                  ),
                                ),
                              )
                            else
                              const Spacer(),
                            ...actions,
                          ],
                        ),
                      ),
                    ),
                  ),
                  if (bottom != null)
                    SizedBox(height: bottomHeight, child: bottom),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
