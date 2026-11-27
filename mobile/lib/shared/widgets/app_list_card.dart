import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/theme.dart';
import 'app_list_inset.dart';
import 'profile_glass_surface.dart';

/// A group of list rows in a rounded container, with an optional [label] above
/// it. Rows inside are hairline-separated and inset to the card rather than the
/// page, via [AppListInset].
class AppListCard extends StatelessWidget {
  const AppListCard({
    super.key,
    this.label,
    this.labelOverArtwork = false,
    this.outlined = false,
    this.profileGlass = false,
    this.dividerIndent,
    this.iconColumnWidth,
    this.clipBehavior = Clip.antiAlias,
    this.verticalPadding = Grid.xxs,
    this.horizontalPadding = Grid.gutter,
    required this.children,
  });

  /// Rendered above the card in sentence case, as written — no uppercasing.
  final String? label;
  final bool labelOverArtwork;
  final bool outlined;
  final bool profileGlass;
  final Clip clipBehavior;

  /// Separator inset from the card edge. Defaults to the standard row label
  /// column, clearing a leading icon. Icon-free cards can pass [_inset] so
  /// separators align with their row content on both sides.
  final double? dividerIndent;
  final double? iconColumnWidth;

  /// Outer padding above and below this section.
  ///
  /// Adjacent cards contribute this padding from both sides. For example,
  /// passing [Grid.twelve] creates a 24dp rhythm between grouped surfaces.
  final double verticalPadding;

  /// Outer horizontal inset; use zero inside an already padded sheet.
  final double horizontalPadding;

  final List<Widget> children;

  static const _inset = Grid.xs;

  @override
  Widget build(BuildContext context) {
    final flat =
        context.dependOnInheritedWidgetOfExactType<FlatListSections>() != null;
    final separated = <Widget>[];
    for (var index = 0; index < children.length; index++) {
      if (index > 0) {
        separated.add(
          Divider(
            height: 1,
            thickness: 1,
            indent: dividerIndent ?? _inset + (iconColumnWidth ?? 22) + Grid.xs,
            endIndent: _inset,
            // The scheme's own border tokens are derived from the page surface,
            // which lands them within a few levels of the card fill — invisible.
            // Tinting with the text color instead keeps the hairline readable on
            // the card in both brightnesses.
            color: context.colors.onSurface.withValues(alpha: 0.07),
          ),
        );
      }
      separated.add(children[index]);
    }

    final card = Material(
      // The softest step on the elevation ramp — a rung below the home tab
      // bar's active pill (primaryContainer, see PR #2810), since a
      // full-width card at the pill's contrast reads heavier than the pill
      // does. The dividers carry the group structure, so the fill only has
      // to separate the card from the page.
      color: flat || profileGlass
          ? Colors.transparent
          : context.colors.surfaceContainerHighest,
      shape: flat
          ? const Border()
          : outlined
          ? RoundedSuperellipseBorder(
              borderRadius: BorderRadius.circular(Radii.container),
              side: BorderSide(
                color: context.colors.inverseSurface.withValues(alpha: .07),
                width: 1,
                strokeAlign: BorderSide.strokeAlignInside,
              ),
            )
          : RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(Radii.container),
            ),
      // Keeps row ripples inside the rounded corners.
      clipBehavior: clipBehavior,
      child: AppListInset(
        horizontal: _inset,
        iconColumnWidth: iconColumnWidth,
        child: Column(mainAxisSize: MainAxisSize.min, children: separated),
      ),
    );
    return Padding(
      padding: EdgeInsets.fromLTRB(
        horizontalPadding,
        verticalPadding,
        horizontalPadding,
        verticalPadding,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (label != null)
            Padding(
              padding: const EdgeInsets.only(left: Grid.half, bottom: Grid.xxs),
              child: ProfileSectionLabel(
                label: label!,
                labelOverArtwork: labelOverArtwork,
              ),
            ),
          if (profileGlass)
            ProfileGlassSurface(
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(Radii.container),
              ),
              dark: Theme.of(context).brightness == Brightness.dark,
              child: card,
            )
          else
            card,
        ],
      ),
    );
  }
}

class ProfileSectionLabel extends StatelessWidget {
  const ProfileSectionLabel({
    super.key,
    required this.label,
    this.labelOverArtwork = true,
    this.title = false,
    this.translucent = false,
    this.textStyle,
    this.center = false,
    this.body = false,
    this.singleLine = false,
    this.fitTextWidth = false,
    this.opaqueTitle = false,
    this.messageTypography = false,
  });
  final String label;
  final TextStyle? textStyle;
  final bool fitTextWidth, opaqueTitle, messageTypography;
  final bool labelOverArtwork, title, translucent, center, body, singleLine;
  @override
  Widget build(BuildContext context) {
    final style =
        textStyle ??
        (title ? context.textTheme.titleLarge! : context.textTheme.labelMedium!)
            .copyWith(
              color: labelOverArtwork
                  ? context.colors.onSurface
                  : context.colors.onSurfaceVariant,
              fontWeight: FontWeight.w600,
            );
    final text = Text(
      label,
      style: style,
      maxLines: singleLine ? 1 : null,
      overflow: singleLine ? TextOverflow.ellipsis : null,
      textAlign: center ? TextAlign.center : TextAlign.start,
    );
    if (opaqueTitle) return Semantics(header: true, child: text);
    if (!labelOverArtwork ||
        defaultTargetPlatform != TargetPlatform.iOS ||
        MediaQuery.highContrastOf(context)) {
      return text;
    }

    final size = MediaQuery.textScalerOf(context).scale(style.fontSize!);
    final dark = Theme.of(context).brightness == Brightness.dark;
    // Secondary vibrancy can lose contrast on mid-tone artwork. Use the
    // primary material with softer ink when half-white would be too faint.
    final surface = context.colors.surface;
    final secondary = Color.alphaBlend(const Color(0x80FFFFFF), surface);
    final strongerSecondary =
        translucent &&
        dark &&
        (secondary.computeLuminance() + .05) /
                (surface.computeLuminance() + .05) <
            4.5;
    final artworkHue = HSLColor.fromColor(surface);
    final glyphTint = messageTypography && artworkHue.saturation > .015
        ? artworkHue.withSaturation(1).withLightness(.5).toColor().toARGB32()
        : null;
    final rtl = Directionality.of(context) == TextDirection.rtl;
    return Semantics(
      header: !body,
      label: label,
      child: ExcludeSemantics(
        child: Stack(
          children: [
            SizedBox(
              width: singleLine || fitTextWidth ? null : double.infinity,
              child: Opacity(opacity: 0, child: text),
            ),
            Positioned.fill(
              child: IgnorePointer(
                child: UiKitView(
                  key: ValueKey((
                    label,
                    size,
                    dark,
                    rtl,
                    translucent,
                    strongerSecondary,
                    body,
                    messageTypography,
                    glyphTint,
                    context.colors.onSurface,
                    center,
                    style.fontWeight,
                    singleLine,
                    style.height,
                  )),
                  hitTestBehavior: PlatformViewHitTestBehavior.transparent,
                  viewType: 'buzz/profile_vibrant_label',
                  // This view draws beneath Flutter tap targets. Let the native sheet pan.
                  gestureBlockingPolicy:
                      UiKitViewGestureBlockingPolicy.doNotBlockGesture,
                  creationParamsCodec: const StandardMessageCodec(),
                  creationParams: {
                    'text': label,
                    'size': size,
                    'dark': dark,
                    'rtl': rtl,
                    'translucent': translucent,
                    'strongerSecondary': strongerSecondary,
                    'body': body,
                    'glyphTint': glyphTint,
                    'inkColor':
                        (messageTypography && !dark
                                ? context.colors.onSurfaceVariant
                                : context.colors.onSurface)
                            .toARGB32(),
                    if (messageTypography) ...{
                      'messageLineHeight': size * style.height!,
                      'tracking': style.letterSpacing,
                    },
                    'center': center,
                    'singleLine': singleLine,
                    'weight': style.fontWeight?.value ?? 400,
                  },
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class FlatListSections extends InheritedWidget {
  const FlatListSections({super.key, required super.child});
  @override
  bool updateShouldNotify(FlatListSections oldWidget) => false;
}
