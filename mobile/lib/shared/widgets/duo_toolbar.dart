import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'ios_navigation_bar.dart';

/// Carries window geometry through the independently sized conversation panes.
class DuoToolbarScope extends InheritedWidget {
  const DuoToolbarScope({
    required this.strip,
    required this.active,
    this.contentInsets = EdgeInsets.zero,
    required super.child,
    super.key,
  });

  final FoldStrip? strip;
  final bool active;
  final EdgeInsets contentInsets;

  static DuoToolbarScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<DuoToolbarScope>();

  @override
  bool updateShouldNotify(DuoToolbarScope oldWidget) =>
      strip != oldWidget.strip ||
      active != oldWidget.active ||
      contentInsets != oldWidget.contentInsets;
}

/// Projects the current route's controls into the window's side strip.
/// The portal retains route ownership and inherited providers for callbacks.
class DuoToolbarPortal extends HookConsumerWidget {
  const DuoToolbarPortal({
    required this.strip,
    required this.active,
    required this.actions,
    required this.child,
    super.key,
  });

  final FoldStrip? strip;
  final bool active;
  final List<IosNavigationAction> actions;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final portal = useMemoized(() => OverlayPortalController()..show());
    return OverlayPortal(
      overlayLocation: OverlayChildLocation.rootOverlay,
      controller: portal,
      overlayChildBuilder: (context) {
        final strip = this.strip;
        if (strip == null || actions.isEmpty) {
          return const SizedBox.shrink();
        }
        final windowSize =
            View.of(context).physicalSize / View.of(context).devicePixelRatio;
        // The package fallback describes the cover display. The inner display's
        // status cluster is shorter; keep this estimated until regions arrive.
        final topClearance =
            strip.isClearanceEstimated && windowSize.shortestSide >= 600
            ? 110.0
            : strip.topClearance;
        return Positioned(
          left:
              IosNavigationMetrics.of(context).railCenter(
                View.of(context).physicalSize /
                    View.of(context).devicePixelRatio,
                strip.rect.center.dx,
              ) -
              strip.width / 2,
          top: strip.rect.top + topClearance,
          width: strip.width,
          child: Offstage(
            offstage: !active,
            child: MediaQuery.removePadding(
              context: context,
              removeTop: true,
              removeBottom: true,
              removeLeft: true,
              removeRight: true,
              child: ConstrainedBox(
                constraints: BoxConstraints(
                  maxHeight:
                      (strip.rect.height -
                              topClearance -
                              strip.bottomClearance -
                              220)
                          .clamp(0.0, double.infinity),
                ),
                child: SingleChildScrollView(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      for (final action in actions)
                        SizedBox(
                          key: ValueKey('duo-toolbar-${action.label}'),
                          height: 60,
                          child: IosNavigationBar(
                            title: '',
                            standaloneControl: true,
                            leading: action,
                          ),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
      },
      child: child,
    );
  }
}

/// Marks sheets whose default Close action is supplied by the Duo rail.
class DuoSheetCloseScope extends InheritedWidget {
  const DuoSheetCloseScope({
    required this.inRail,
    required super.child,
    super.key,
  });

  final bool inRail;

  static bool of(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<DuoSheetCloseScope>()
          ?.inRail ??
      false;

  @override
  bool updateShouldNotify(DuoSheetCloseScope oldWidget) =>
      inRail != oldWidget.inRail;
}
