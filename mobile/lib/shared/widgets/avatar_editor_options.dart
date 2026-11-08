import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'avatar_editor_options_scope.dart';
import 'duo_toolbar.dart';
import 'ios_navigation_bar.dart';

/// Keeps phone options horizontal and projects Edit Photo options into Duo's rail.
class AvatarEditorOptions extends HookWidget {
  const AvatarEditorOptions({
    required this.options,
    this.centered = true,
    this.dividerBefore,
    super.key,
  });

  final List<Widget> options;
  final bool centered;
  final int? dividerBefore;

  @override
  Widget build(BuildContext context) {
    final scope = AvatarEditorOptionsScope.maybeOf(context);
    final toolbar = DuoToolbarScope.maybeOf(context);
    final strip = toolbar?.strip ?? FoldGeometry.of(context).strip;
    final inRail =
        scope != null &&
        strip != null &&
        defaultTargetPlatform == TargetPlatform.iOS;
    final portal = useMemoized(() => OverlayPortalController()..show());
    if (!inRail) {
      return Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (centered) const Spacer(),
          for (var i = 0; i < options.length; i++) ...[
            if (i > 0) const SizedBox(width: Grid.half),
            if (i == dividerBefore) ...[
              Container(
                width: 1,
                height: 64,
                color: context.colors.outlineVariant,
              ),
              const SizedBox(width: Grid.half),
            ],
            Expanded(child: options[i]),
          ],
          if (centered) const Spacer(),
        ],
      );
    }
    final active =
        (ModalRoute.of(context)?.isCurrent ?? true) &&
        (toolbar?.active ?? true);
    final window = MediaQueryData.fromView(View.of(context));
    final topClearance =
        strip.isClearanceEstimated && window.size.shortestSide >= 600
        ? duoInnerDisplayEstimatedRailClearance
        : strip.topClearance;
    return OverlayPortal(
      overlayLocation: OverlayChildLocation.rootOverlay,
      controller: portal,
      overlayChildBuilder: (context) => Positioned(
        left:
            IosNavigationMetrics.of(
              context,
            ).railCenter(window.size, strip.rect.center.dx) -
            strip.width / 2,
        width: strip.width,
        top: strip.rect.top + topClearance + 120,
        bottom: window.viewPadding.bottom + Grid.xxs,
        child: Offstage(
          offstage: !active,
          child: IgnorePointer(
            ignoring: !scope.interactive,
            child: MediaQuery.removePadding(
              context: context,
              removeTop: true,
              removeBottom: true,
              child: Align(
                alignment: Alignment.bottomCenter,
                child: SingleChildScrollView(
                  child: Column(
                    key: const ValueKey('avatar-editor-side-options'),
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      for (var i = 0; i < options.length; i++) ...[
                        if (i > 0) const SizedBox(height: Grid.xxs),
                        options[i],
                      ],
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
      child: const SizedBox.shrink(),
    );
  }
}
