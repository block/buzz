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

/// Height for a compact title rail below the app bar's action row.
///
/// The rail normally stays at 40dp, but grows with an accessible title rather
/// than clipping text at larger system text sizes.
double frostedAppBarLowerTitleHeight(
  BuildContext context, {
  TextStyle? titleStyle,
}) {
  final style = _effectiveTitleStyle(context, titleStyle);
  final scaledFontSize = MediaQuery.textScalerOf(
    context,
  ).scale(style.fontSize ?? 20);
  final titleHeight = scaledFontSize * (style.height ?? 1);
  return titleHeight > 40 ? titleHeight : 40;
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

  /// Extends [bottom] upward into the title row without moving the app bar's
  /// outer bounds. This keeps overlapping controls inside the app bar's hit
  /// test region as well as its paint region.
  final double bottomOverlap;

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

  /// Whether to apply the translucent blur treatment behind the app bar.
  ///
  /// A page can leave its painted backdrop exposed at rest, then turn this on
  /// when scrolling moves content beneath the controls.
  final bool frosted;

  /// Opacity of the frosted surface above the blurred backdrop.
  final double frostedSurfaceOpacity;

  /// Blur strength of the frosted backdrop.
  final double frostedBlurSigma;

  /// Whether to draw a divider below the app bar.
  final bool showBottomDivider;

  /// Opacity of the divider below the app bar.
  final double bottomDividerOpacity;

  const FrostedAppBar({
    super.key,
    this.leading,
    this.automaticallyImplyLeading = true,
    this.title,
    this.titleStyle,
    this.titleContentHeight = 0,
    this.bottom,
    this.bottomHeight = 0,
    this.bottomOverlap = 0,
    this.actions = const [],
    this.horizontalInset = Grid.quarter,
    this.iconColor,
    this.gradient,
    this.frosted = true,
    this.frostedSurfaceOpacity = 0.5,
    this.frostedBlurSigma = 20,
    this.showBottomDivider = true,
    this.bottomDividerOpacity = 0.15,
  }) : assert(bottom == null || bottomHeight > 0),
       assert(bottomOverlap >= 0),
       assert(bottom != null || bottomOverlap == 0),
       assert(frostedBlurSigma >= 0),
       assert(bottomDividerOpacity >= 0 && bottomDividerOpacity <= 1);

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

    final titleRow = SizedBox(
      height: barContentHeight,
      child: Padding(
        padding: EdgeInsets.symmetric(horizontal: horizontalInset),
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
                          : horizontalInset < Grid.gutter
                          ? Grid.gutter - horizontalInset
                          : 0,
                      right: actions.isEmpty
                          ? horizontalInset < Grid.gutter
                                ? Grid.gutter - horizontalInset
                                : 0
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
    );
    final contentBody = bottom != null && bottomOverlap > 0
        ? SizedBox(
            height: barContentHeight + bottomHeight,
            child: Stack(
              children: [
                Positioned(top: 0, left: 0, right: 0, child: titleRow),
                Positioned(
                  top: barContentHeight - bottomOverlap,
                  left: 0,
                  right: 0,
                  height: bottomHeight + bottomOverlap,
                  child: bottom!,
                ),
              ],
            ),
          )
        : Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              titleRow,
              if (bottom != null) SizedBox(height: bottomHeight, child: bottom),
            ],
          );

    final content = DirectionalTransitionMotion(
      transformKey: const ValueKey(
        'frosted-app-bar-content-transition-transform',
      ),
      opacityKey: const ValueKey('frosted-app-bar-content-transition-opacity'),
      child: contentBody,
    );

    final background = Container(
      key: const ValueKey('frosted-app-bar-background'),
      padding: EdgeInsets.only(top: topPadding),
      decoration: BoxDecoration(
        color: !frosted
            ? Colors.transparent
            : gradient == null
            ? context.colors.surface.withValues(alpha: frostedSurfaceOpacity)
            : null,
        gradient: gradient,
        border: showBottomDivider
            ? Border(
                bottom: BorderSide(
                  color: navigationDivider(context, bottomDividerOpacity),
                  width: _kBottomBorderWidth,
                ),
              )
            : null,
      ),
      child: content,
    );

    final child = ClipRect(
      child: frosted
          ? BackdropFilter(
              filter: ImageFilter.blur(
                sigmaX: frostedBlurSigma,
                sigmaY: frostedBlurSigma,
              ),
              child: background,
            )
          : background,
    );

    return Positioned(top: 0, left: 0, right: 0, child: child);
  }
}
