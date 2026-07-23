import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';

/// Height of the frosted app bar content area below the safe area.
const _kBarContentHeight = Grid.xxs + 32 + Grid.xxs; // 48

/// Returns the total height of the [FrostedAppBar] including safe area padding.
///
/// Use this to add top spacing to body content so it starts below the bar.
double frostedAppBarHeight(BuildContext context) {
  return MediaQuery.paddingOf(context).top + _kBarContentHeight;
}

/// A frosted-glass floating app bar designed to sit inside a [Stack].
///
/// Renders as a [Positioned] widget pinned to the top of its parent Stack.
/// Content scrolls underneath with a translucent backdrop blur effect.
class FrostedAppBar extends StatelessWidget {
  /// Widget displayed on the leading (left) side. If null and the navigator
  /// can pop, a back button is shown automatically.
  final Widget? leading;

  /// Widget displayed in the center/title area.
  final Widget? title;

  /// Widgets displayed on the trailing (right) side.
  final List<Widget> actions;

  const FrostedAppBar({
    super.key,
    this.leading,
    this.title,
    this.actions = const [],
  });

  @override
  Widget build(BuildContext context) {
    final topPadding = MediaQuery.paddingOf(context).top;
    final canPop = Navigator.canPop(context);

    final effectiveLeading =
        leading ??
        (canPop
            ? SizedBox(
                width: 48,
                height: 48,
                child: IconButton(
                  onPressed: () => Navigator.of(context).pop(),
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
            padding: EdgeInsets.only(top: topPadding),
            decoration: BoxDecoration(
              color: context.colors.surface.withValues(alpha: 0.5),
              border: Border(
                bottom: BorderSide(
                  color: context.colors.outlineVariant.withValues(alpha: 0.3),
                ),
              ),
            ),
            child: SizedBox(
              height: _kBarContentHeight,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.quarter),
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
                            style: context.textTheme.titleMedium?.copyWith(
                              fontWeight: FontWeight.w600,
                            ),
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
        ),
      ),
    );
  }
}
