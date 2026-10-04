import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';
import 'buzz_navigation_metrics.dart';
import 'directional_transition_scope.dart';
import 'frosted_scroll_under_scope.dart';
import 'ios_glass_navigation_button.dart';
import 'ios_glass_navigation_action.dart';
import 'ios_navigation_bar.dart';
export 'ios_navigation_bar.dart' show IosNavigationAction;

/// Minimum height of the frosted app bar content area below the safe area.
const _kBarContentMinHeight = buzzNavigationRowHeight;
const _kBottomBorderWidth = 1.0;

TextStyle _effectiveTitleStyle(BuildContext context, TextStyle? titleStyle) {
  final baseStyle =
      context.textTheme.titleSmall ??
      const TextStyle(fontSize: 16, height: 1.4);
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
  ).scale(style.fontSize ?? 16);
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
  ).scale(style.fontSize ?? 16);
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
  bool nativeLargeTitle = false,
}) {
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    return MediaQuery.paddingOf(context).top +
        IosNavigationMetrics.of(context).compactHeight +
        (nativeLargeTitle
            ? IosNavigationMetrics.of(context).largeTitleHeight
            : 0) +
        bottomHeight;
  }
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

  /// Whether [title] is centered in the full bar rather than flowing after the
  /// leading control. Page titles should keep the default; identity-style
  /// headers can opt out.
  final bool centerTitle;

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

  /// Explicit UIKit title for headers whose Flutter title is a custom widget.
  final String? nativeTitle;

  /// Optional second line and action for a native channel title.
  final String? nativeSubtitle;

  /// Existing retention disclosure displayed by the native conversation title.
  final String? nativeEphemeralLabel;

  /// Reactive counterpart avatar and presence badge for a native DM title.
  final IosNavigationAction? nativeTitleAvatar;
  final Color? nativeTitlePresenceColor;
  final VoidCallback? onNativeTitlePressed;

  /// Whether UIKit should expand the title at the top of the page.
  final bool nativeLargeTitle;

  /// Duration for coordinated native header and overlapping control movement.
  final Duration nativeLayoutDuration;

  /// Native replacement for a custom leading widget.
  final IosNavigationAction? nativeLeading;

  /// Native replacements for composite Flutter actions, including menus.
  final List<IosNavigationAction>? nativeActions;

  /// Uses a composable Flutter header while a backdrop covers the native view.
  final ValueListenable<bool>? nativeViewSuppressed;

  const FrostedAppBar({
    super.key,
    this.nativeTitle,
    this.nativeSubtitle,
    this.nativeEphemeralLabel,
    this.nativeTitleAvatar,
    this.nativeTitlePresenceColor,
    this.onNativeTitlePressed,
    this.nativeLargeTitle = false,
    this.nativeLayoutDuration = Duration.zero,
    this.nativeLeading,
    this.nativeActions,
    this.nativeViewSuppressed,
    this.leading,
    this.automaticallyImplyLeading = true,
    this.title,
    this.centerTitle = false,
    this.titleStyle,
    this.titleContentHeight = 0,
    this.bottom,
    this.bottomHeight = 0,
    this.bottomOverlap = 0,
    this.actions = const [],
    this.horizontalInset = Grid.xxs,
    this.iconColor,
    this.gradient,
    this.frosted = true,
    this.frostedSurfaceOpacity = 0.5,
    this.frostedBlurSigma = 20,
    this.showBottomDivider = true,
    this.bottomDividerOpacity = 0.05,
  }) : assert(bottom == null || bottomHeight > 0),
       assert(bottomOverlap >= 0),
       assert(bottom != null || bottomOverlap == 0),
       assert(frostedBlurSigma >= 0),
       assert(bottomDividerOpacity >= 0 && bottomDividerOpacity <= 1);

  @override
  Widget build(BuildContext context) {
    final suppression = nativeViewSuppressed;
    if (defaultTargetPlatform == TargetPlatform.iOS && suppression != null) {
      return ValueListenableBuilder<bool>(
        valueListenable: suppression,
        builder: (context, suppressed, _) => _build(context, suppressed),
      );
    }
    return _build(context, false);
  }

  Widget _build(BuildContext context, bool nativeSuppressed) {
    if (defaultTargetPlatform == TargetPlatform.iOS && !nativeSuppressed) {
      final offset = IosNavigationScrollScope.maybeOf(context);
      Widget buildNative(double scrollOffset) {
        // Scrolling tracks the finger directly; only the search focus change
        // uses the coordinated layout transition.
        final layoutDuration = nativeLargeTitle && scrollOffset > 0
            ? Duration.zero
            : nativeLayoutDuration;
        final extra = nativeLargeTitle
            ? (IosNavigationMetrics.of(context).largeTitleHeight - scrollOffset)
                  .clamp(0.0, IosNavigationMetrics.of(context).largeTitleHeight)
            : 0.0;
        final barHeight =
            MediaQuery.paddingOf(context).top +
            IosNavigationMetrics.of(context).compactHeight +
            extra;
        return AnimatedPositioned(
          duration: layoutDuration,
          curve: Curves.easeInOutCubic,
          top: 0,
          left: 0,
          right: 0,
          height: barHeight + bottomHeight,
          child: Stack(
            children: [
              Positioned(
                top: 0,
                left: 0,
                right: 0,
                height: barHeight,
                child: IosNavigationBar(
                  title:
                      nativeTitle ??
                      (title is Text ? (title as Text).data ?? '' : ''),
                  subtitle: nativeSubtitle,
                  ephemeralLabel: nativeEphemeralLabel,
                  titleAvatar: nativeTitleAvatar,
                  titlePresenceColor: nativeTitlePresenceColor,
                  onTitlePressed: onNativeTitlePressed,
                  largeTitle: nativeLargeTitle,
                  foregroundColor: iconColor,
                  leading: nativeLeading ?? _nativeAction(leading),
                  onBack:
                      automaticallyImplyLeading &&
                          leading == null &&
                          Navigator.canPop(context)
                      ? () => Navigator.of(context).maybePop()
                      : null,
                  actions:
                      nativeActions ??
                      actions
                          .map(_nativeAction)
                          .whereType<IosNavigationAction>()
                          .toList(),
                ),
              ),
              if (bottom != null)
                AnimatedPositioned(
                  duration: layoutDuration,
                  curve: Curves.easeInOutCubic,
                  top: barHeight - bottomOverlap,
                  left: 0,
                  right: 0,
                  height: bottomHeight + bottomOverlap,
                  child: bottomOverlap > 0 || bottom is SizedBox
                      ? bottom!
                      : ColoredBox(
                          color: context.colors.surface,
                          child: bottom!,
                        ),
                ),
            ],
          ),
        );
      }

      return offset == null
          ? buildNative(0)
          : ValueListenableBuilder<double>(
              valueListenable: offset,
              builder: (context, value, child) => buildNative(value),
            );
    }
    final topPadding = MediaQuery.paddingOf(context).top;
    final scrollUnder = FrostedScrollUnderScope.maybeOf(context);
    final paintsBottomDivider =
        showBottomDivider && (scrollUnder?.isScrolledUnder ?? true);
    final canPop = Navigator.canPop(context);
    final effectiveTitleStyle = _effectiveTitleStyle(context, titleStyle);
    final barContentHeight = _barContentHeight(
      context,
      titleStyle,
      titleContentHeight,
    );
    final usesAutomaticIosGlassBackButton =
        !nativeSuppressed &&
        leading == null &&
        automaticallyImplyLeading &&
        canPop &&
        Theme.of(context).platform == TargetPlatform.iOS;
    final effectiveIconColor = iconColor ?? context.colors.primary;

    final effectiveLeading =
        leading ??
        (automaticallyImplyLeading && canPop
            ? usesAutomaticIosGlassBackButton
                  ? IosGlassNavigationButton(
                      icon: IosGlassNavigationIcon.back,
                      semanticLabel: 'Back',
                      onPressed: () => Navigator.of(context).maybePop(),
                      width: iosGlassChannelHeaderLeadingWidth,
                      buttonCenterX: iosGlassChannelHeaderButtonCenterX,
                      foregroundColor: effectiveIconColor,
                    )
                  : SizedBox(
                      width: 48,
                      height: 48,
                      child: IconButton(
                        onPressed: () => Navigator.of(context).pop(),
                        color: effectiveIconColor,
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
          data: IconThemeData(color: effectiveIconColor),
          child: _CenteredNavigationLayout(
            leading: effectiveLeading,
            title: title == null
                ? null
                : DefaultTextStyle.merge(
                    style: effectiveTitleStyle,
                    overflow: TextOverflow.ellipsis,
                    maxLines: 1,
                    textAlign: centerTitle ? TextAlign.center : TextAlign.start,
                    child: centerTitle
                        ? title!
                        : Padding(
                            padding: EdgeInsets.only(
                              left: effectiveLeading != null
                                  ? usesAutomaticIosGlassBackButton
                                        ? iosGlassChannelHeaderTitleSpacing
                                        : 0
                                  : horizontalInset < Grid.gutter
                                  ? Grid.gutter - horizontalInset
                                  : 0,
                              right:
                                  actions.isEmpty &&
                                      horizontalInset < Grid.gutter
                                  ? Grid.gutter - horizontalInset
                                  : 0,
                            ),
                            child: title!,
                          ),
                  ),
            actions: actions.isEmpty
                ? null
                : Row(mainAxisSize: MainAxisSize.min, children: actions),
            centered: centerTitle,
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
                  color: paintsBottomDivider
                      ? navigationDivider(context, bottomDividerOpacity)
                      : Colors.transparent,
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

enum _NavigationSlot { leading, title, actions }

class _CenteredNavigationLayout extends StatelessWidget {
  const _CenteredNavigationLayout({
    this.leading,
    this.title,
    this.actions,
    required this.centered,
  });

  final Widget? leading;
  final Widget? title;
  final Widget? actions;
  final bool centered;

  @override
  Widget build(BuildContext context) {
    if (!centered) {
      return Row(
        children: [
          ?leading,
          if (title != null) Expanded(child: title!) else const Spacer(),
          ?actions,
        ],
      );
    }
    return CustomMultiChildLayout(
      delegate: _CenteredNavigationLayoutDelegate(),
      children: [
        if (leading != null)
          LayoutId(id: _NavigationSlot.leading, child: leading!),
        if (title != null) LayoutId(id: _NavigationSlot.title, child: title!),
        if (actions != null)
          LayoutId(id: _NavigationSlot.actions, child: actions!),
      ],
    );
  }
}

class _CenteredNavigationLayoutDelegate extends MultiChildLayoutDelegate {
  @override
  void performLayout(Size size) {
    Size leadingSize = Size.zero;
    if (hasChild(_NavigationSlot.leading)) {
      leadingSize = layoutChild(
        _NavigationSlot.leading,
        BoxConstraints.loose(size),
      );
      positionChild(
        _NavigationSlot.leading,
        Offset(0, (size.height - leadingSize.height) / 2),
      );
    }

    Size actionsSize = Size.zero;
    if (hasChild(_NavigationSlot.actions)) {
      actionsSize = layoutChild(
        _NavigationSlot.actions,
        BoxConstraints.loose(size),
      );
      positionChild(
        _NavigationSlot.actions,
        Offset(
          size.width - actionsSize.width,
          (size.height - actionsSize.height) / 2,
        ),
      );
    }

    if (hasChild(_NavigationSlot.title)) {
      final occupiedSideWidth = leadingSize.width > actionsSize.width
          ? leadingSize.width
          : actionsSize.width;
      final sideWidth = occupiedSideWidth == 0
          ? 0.0
          : occupiedSideWidth + Grid.xs;
      final titleWidth = (size.width - sideWidth * 2).clamp(0.0, size.width);
      final titleSize = layoutChild(
        _NavigationSlot.title,
        BoxConstraints(maxWidth: titleWidth, maxHeight: size.height),
      );
      positionChild(
        _NavigationSlot.title,
        Offset(
          (size.width - titleSize.width) / 2,
          (size.height - titleSize.height) / 2,
        ),
      );
    }
  }

  @override
  bool shouldRelayout(
    covariant _CenteredNavigationLayoutDelegate oldDelegate,
  ) => false;
}

IosNavigationAction? _nativeAction(Widget? widget) {
  if (widget is Padding) return _nativeAction(widget.child);
  if (widget is SizedBox) return _nativeAction(widget.child);
  if (widget is IosGlassNavigationAction) {
    return IosNavigationAction(
      label: widget.label,
      onPressed: widget.isBusy ? null : widget.onPressed,
    );
  }
  if (widget is IosGlassNavigationButton) {
    final symbol = switch (widget.icon) {
      IosGlassNavigationIcon.back => 'chevron.backward',
      IosGlassNavigationIcon.close => 'xmark',
      IosGlassNavigationIcon.camera => 'camera',
      IosGlassNavigationIcon.photoLibrary => 'photo.on.rectangle',
      IosGlassNavigationIcon.palette => 'paintpalette',
      IosGlassNavigationIcon.droplet => 'drop',
      IosGlassNavigationIcon.emoji => 'face.smiling',
      IosGlassNavigationIcon.person => 'person',
      IosGlassNavigationIcon.frame => 'crop',
      IosGlassNavigationIcon.rotateCamera =>
        'arrow.triangle.2.circlepath.camera',
      IosGlassNavigationIcon.shutter => 'circle',
      IosGlassNavigationIcon.colorSwatch => 'circle.fill',
      IosGlassNavigationIcon.sun => 'sun.max',
      IosGlassNavigationIcon.moon => 'moon',
      IosGlassNavigationIcon.systemAppearance => 'circle.lefthalf.filled',
    };
    return IosNavigationAction(
      label: widget.label ?? widget.semanticLabel,
      symbol: widget.label == null ? symbol : null,
      selected: widget.isSelected,
      onPressed: widget.isBusy ? null : widget.onPressed,
    );
  }
  if (widget is IconButton) {
    final icon = widget.icon is Icon ? (widget.icon as Icon).icon : null;
    final symbol = icon == LucideIcons.x
        ? 'xmark'
        : icon == LucideIcons.chevronLeft || icon == LucideIcons.arrowLeft
        ? 'chevron.backward'
        : icon == LucideIcons.users
        ? 'person.2'
        : 'ellipsis';
    return IosNavigationAction(
      label: widget.tooltip ?? 'Action',
      symbol: symbol,
      onPressed: widget.onPressed,
    );
  }
  if (widget is ButtonStyleButton && widget.child is Text) {
    return IosNavigationAction(
      label: (widget.child as Text).data ?? '',
      onPressed: widget.onPressed,
    );
  }
  return null;
}
