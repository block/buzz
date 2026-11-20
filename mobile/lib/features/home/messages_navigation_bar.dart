import 'dart:async';

import '../../shared/widgets/agent_navigation_overlay.dart';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/navigation_button_surface.dart';
import '../../shared/widgets/android_expanding_menu_button.dart';
import '../../shared/widgets/ios_message_create_button.dart';

/// Separate glass shortcuts for search and creation.
/// Size and spacing of Duo's native rail controls (`DuoToolbarPortal`).
const double _railControlSize = 48;
const double _railControlGap = 12;

class MessagesNavigationBar extends HookWidget {
  const MessagesNavigationBar({
    super.key,
    required this.onSearch,
    required this.creationActions,
    required this.agent,
    this.onRecordingChanged,
    this.axis = Axis.horizontal,
  });
  final Axis axis;

  /// Side of each control in the vertical layout, which keeps them on the
  /// right edge of its 256-point width.
  static const double railControlSize = _railControlSize;
  final VoidCallback onSearch;
  final Widget agent;
  final ValueChanged<bool>? onRecordingChanged;
  final List<({String label, String symbol, VoidCallback? onPressed})>
  creationActions;

  @override
  Widget build(BuildContext context) {
    final menuOpen = useState(false);
    final menuMoving = useState(false);
    final disableAnimations = MediaQuery.disableAnimationsOf(context);
    void menuVisibilityChanged(bool visible) {
      if (menuOpen.value == visible) return;
      menuMoving.value = !disableAnimations;
      menuOpen.value = visible;
    }

    useEffect(() {
      menuOpen.value = false;
      menuMoving.value = false;
      return null;
    }, [axis]);
    final overlay = useState(false);
    Widget hideDuringVoice(Widget child) => ExcludeSemantics(
      excluding: overlay.value,
      child: IgnorePointer(
        ignoring: overlay.value,
        child: Opacity(opacity: overlay.value ? 0 : 1, child: child),
      ),
    );
    // Duo's rail stacks these at the size and spacing of its native rail
    // controls.
    final vertical = axis == Axis.vertical;
    final controlSize = vertical
        ? _railControlSize
        : AgentNavigationOverlay.defaultControlSize;
    return AgentNavigationOverlay(
      active: overlay,
      menuOpen: menuOpen.value || menuMoving.value,
      onRecordingChanged: onRecordingChanged,
      controlSize: controlSize,
      child: Builder(
        builder: (context) {
          if (vertical) {
            const height = _railControlSize * 3 + _railControlGap * 2;
            return SizedBox(
              width: 256,
              height: height,
              child: Stack(
                clipBehavior: Clip.none,
                children: [
                  Positioned(
                    right: 0,
                    child: hideDuringVoice(
                      IosMessageCreateButton(
                        key: const ValueKey('vertical-create-menu'),
                        mergingNavigation: true,
                        navigationAxis: Axis.vertical,
                        onMenuVisibilityChanged: menuVisibilityChanged,
                        actions: creationActions,
                        dividerBefore: const {2},
                        sectionTitles: const {0: 'Solo', 2: 'Team'},
                        accessibilityLabel: 'New',
                        width: _railControlSize,
                        height: height,
                        navigationControlSize: _railControlSize,
                        navigationGap: _railControlGap,
                        plain: false,
                      ),
                    ),
                  ),
                  Positioned(
                    right: 0,
                    top: 0,
                    width: _railControlSize,
                    height: _railControlSize + _railControlGap / 2,
                    child: ClipRect(
                      child: Align(
                        alignment: Alignment.topCenter,
                        child: _MenuNudge(
                          axis: Axis.vertical,
                          offset: menuOpen.value
                              ? _railControlSize + _railControlGap
                              : 0,
                          menuOpen: menuOpen.value,
                          alignment: Alignment.center,
                          child: hideDuringVoice(
                            _search(context, _railControlSize),
                          ),
                        ),
                      ),
                    ),
                  ),
                  Positioned(
                    bottom: 0,
                    right: 0,
                    width: 256,
                    height: _railControlSize,
                    child: OverflowBox(
                      alignment: Alignment.centerRight,
                      minWidth: 0,
                      maxWidth: 256,
                      child: Align(
                        widthFactor: 1,
                        alignment: Alignment.centerRight,
                        child: _MenuNudge(
                          axis: Axis.vertical,
                          offset: menuOpen.value
                              ? -_railControlSize - _railControlGap
                              : 0,
                          menuOpen: menuOpen.value,
                          alignment: Alignment.center,
                          onEnd: () => menuMoving.value = false,
                          child: NavigationGlassGroup(child: agent),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            );
          }
          if (defaultTargetPlatform == TargetPlatform.iOS) {
            return SizedBox(
              width: 256,
              height: 56,
              child: Stack(
                clipBehavior: Clip.none,
                children: [
                  hideDuringVoice(
                    IosMessageCreateButton(
                      key: const ValueKey('horizontal-create-menu'),
                      mergingNavigation: true,
                      onMenuVisibilityChanged: menuVisibilityChanged,
                      actions: creationActions,
                      dividerBefore: const {2},
                      sectionTitles: const {0: 'Solo', 2: 'Team'},
                      label: 'New',
                      showIconWithLabel: true,
                      accessibilityLabel: 'New',
                      width: 256,
                      height: 56,
                      plain: false,
                    ),
                  ),
                  Positioned(
                    left: 0,
                    width: 72,
                    top: 0,
                    bottom: 0,
                    child: ClipRect(
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: _MenuNudge(
                          offset: menuOpen.value ? 100 : 0,
                          menuOpen: menuOpen.value,
                          alignment: Alignment.center,
                          child: hideDuringVoice(_search(context)),
                        ),
                      ),
                    ),
                  ),
                  Positioned(
                    right: 0,
                    width: 256,
                    top: 0,
                    bottom: 0,
                    child: OverflowBox(
                      alignment: Alignment.centerRight,
                      minWidth: 0,
                      maxWidth: 256,
                      child: Align(
                        widthFactor: 1,
                        alignment: Alignment.centerRight,
                        child: _MenuNudge(
                          offset: menuOpen.value ? -100 : 0,
                          menuOpen: menuOpen.value,
                          alignment: Alignment.center,
                          onEnd: () => menuMoving.value = false,
                          child: NavigationGlassGroup(child: agent),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            );
          }
          return SizedBox(
            width: 224 + Grid.xs * 2,
            height: 56,
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                hideDuringVoice(
                  Builder(
                    builder: (controlsContext) => Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        NavigationButtonSurface(child: _search(context)),
                        const SizedBox(width: Grid.xs),
                        AndroidExpandingMenuButton(
                          expansionBoundsContext: controlsContext,
                          actions: creationActions,
                          dividerBefore: const {2},
                          sectionTitles: const {0: 'Solo', 2: 'Team'},
                        ),
                        const SizedBox(width: Grid.xs),
                        const SizedBox.square(dimension: 56),
                      ],
                    ),
                  ),
                ),
                Positioned(right: 0, top: 0, child: agent),
              ],
            ),
          );
        },
      ),
    );
  }

  Widget _search(
    BuildContext context, [
    double size = AgentNavigationOverlay.defaultControlSize,
  ]) => SizedBox.square(
    dimension: size,
    child: IconButton(
      tooltip: 'Search',
      onPressed: () {
        unawaited(HapticFeedback.lightImpact());
        onSearch();
      },
      color: context.colors.onSurface,
      icon: const Icon(BuzzIcons.search300, size: 24),
    ),
  );
}

class _MenuNudge extends StatelessWidget {
  const _MenuNudge({
    required this.offset,
    this.axis = Axis.horizontal,
    required this.menuOpen,
    required this.alignment,
    required this.child,
    this.onEnd,
  });
  final VoidCallback? onEnd;
  final double offset;
  final Axis axis;
  final bool menuOpen;
  final Alignment alignment;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 180);
    final scale = menuOpen ? .9 : 1.0;
    return IgnorePointer(
      ignoring: menuOpen,
      child: ExcludeSemantics(
        excluding: menuOpen,
        child: AnimatedContainer(
          onEnd: onEnd,
          duration: duration,
          curve: Curves.easeOutCubic,
          transformAlignment: alignment,
          transform: Matrix4.translationValues(
            axis == Axis.horizontal ? offset : 0,
            axis == Axis.vertical ? offset : 0,
            0,
          )..scaleByDouble(scale, scale, 1, 1),
          child: child,
        ),
      ),
    );
  }
}
