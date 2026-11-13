import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'ios_navigation_bar.dart';

/// Rail top clearance on Duo's inner display until UIKit reports its regions.
/// The package estimate describes the cover display's taller status cluster.
const double duoInnerDisplayEstimatedRailClearance = 110;

/// Whether Duo controls should use the compact cover arrangement.
bool isCompactDuoWindow(BuildContext context) {
  // Subscribe to resizing even inside a pane whose MediaQuery is narrower.
  MediaQuery.sizeOf(context);
  final view = View.of(context);
  return (view.physicalSize / view.devicePixelRatio).shortestSide < 600;
}

/// Carries window geometry through the independently sized conversation panes.
class DuoToolbarScope extends InheritedModel<Object> {
  const DuoToolbarScope({
    required this.strip,
    required this.active,
    this.contentInsets = EdgeInsets.zero,
    this.leadingSlots = 0,
    required super.child,
    super.key,
  });

  final FoldStrip? strip;
  final bool active;
  final EdgeInsets contentInsets;

  /// Space above detail actions for the workspace's persistent rail controls.
  final int leadingSlots;

  static const Object _stripAspect = #strip;

  static DuoToolbarScope? maybeOf(BuildContext context) =>
      InheritedModel.inheritFrom<DuoToolbarScope>(context);

  /// The enclosing pane's [strip], depending on nothing else.
  ///
  /// [active] flips whenever a route covers the pane, so geometry readers
  /// such as headers and page lists use this to avoid rebuilding with it.
  static FoldStrip? maybeStripOf(BuildContext context) =>
      InheritedModel.inheritFrom<DuoToolbarScope>(
        context,
        aspect: _stripAspect,
      )?.strip;

  @override
  bool updateShouldNotify(DuoToolbarScope oldWidget) =>
      strip != oldWidget.strip ||
      active != oldWidget.active ||
      contentInsets != oldWidget.contentInsets ||
      leadingSlots != oldWidget.leadingSlots;

  @override
  bool updateShouldNotifyDependent(
    DuoToolbarScope oldWidget,
    Set<Object> dependencies,
  ) => strip != oldWidget.strip;
}

/// Content clearance for a pane, or a full-window route outside a pane.
EdgeInsets duoContentInsetsOf(BuildContext context) {
  final pane = DuoToolbarScope.maybeOf(context);
  if (pane != null) return pane.contentInsets;
  final strip = FoldGeometry.of(context).strip;
  return EdgeInsets.only(
    left: strip?.side == FoldStripSide.left ? strip!.width : 0,
    right: strip?.side == FoldStripSide.right ? strip!.width : 0,
  );
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
        final metrics = IosNavigationMetrics.of(context);
        final measuredTop = metrics.windowSize == windowSize
            ? metrics.railTopClearance
            : null;
        final actionOffset =
            (DuoToolbarScope.maybeOf(context)?.leadingSlots ?? 0) * 60.0;
        final topClearance =
            (measuredTop != null
                ? measuredTop + 12
                : strip.isClearanceEstimated && windowSize.shortestSide >= 600
                ? duoInnerDisplayEstimatedRailClearance
                : strip.topClearance) +
            actionOffset;
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
