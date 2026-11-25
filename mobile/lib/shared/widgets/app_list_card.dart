import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/theme.dart';
import 'app_list_inset.dart';

/// A group of list rows in a rounded container, with an optional [label] above
/// it. Rows inside are hairline-separated and inset to the card rather than the
/// page, via [AppListInset].
class AppListCard extends StatelessWidget {
  const AppListCard({
    super.key,
    this.label,
    this.labelOverArtwork = false,
    this.outlined = false,
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

  Widget _sectionLabel(BuildContext context) {
    final style = context.textTheme.labelMedium!.copyWith(
      color: labelOverArtwork
          ? context.colors.onSurface
          : context.colors.onSurfaceVariant,
      fontWeight: FontWeight.w600,
    );
    final text = Text(label!, style: style);
    if (!labelOverArtwork ||
        defaultTargetPlatform != TargetPlatform.iOS ||
        MediaQuery.highContrastOf(context)) {
      return text;
    }

    final size = MediaQuery.textScalerOf(context).scale(style.fontSize!);
    final dark = Theme.of(context).brightness == Brightness.dark;
    final rtl = Directionality.of(context) == TextDirection.rtl;
    return Semantics(
      header: true,
      label: label,
      child: ExcludeSemantics(
        child: Stack(
          children: [
            SizedBox(
              width: double.infinity,
              child: Opacity(opacity: 0, child: text),
            ),
            Positioned.fill(
              child: IgnorePointer(
                child: UiKitView(
                  key: ValueKey((label, size, dark, rtl)),
                  viewType: 'buzz/profile_vibrant_label',
                  creationParamsCodec: const StandardMessageCodec(),
                  creationParams: {
                    'text': label,
                    'size': size,
                    'dark': dark,
                    'rtl': rtl,
                  },
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
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
              child: _sectionLabel(context),
            ),
          Material(
            // The softest step on the elevation ramp — a rung below the home tab
            // bar's active pill (primaryContainer, see PR #2810), since a
            // full-width card at the pill's contrast reads heavier than the pill
            // does. The dividers carry the group structure, so the fill only has
            // to separate the card from the page.
            color: context.colors.surfaceContainerHighest,
            shape: outlined
                ? RoundedSuperellipseBorder(
                    borderRadius: BorderRadius.circular(Radii.container),
                    side: BorderSide(
                      color: context.colors.inverseSurface.withValues(
                        alpha: .07,
                      ),
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
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: separated,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
