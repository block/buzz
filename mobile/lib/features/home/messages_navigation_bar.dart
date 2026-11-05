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
class MessagesNavigationBar extends HookWidget {
  const MessagesNavigationBar({
    super.key,
    required this.onSearch,
    required this.creationActions,
    required this.agent,
    this.onRecordingChanged,
  });
  final VoidCallback onSearch;
  final Widget agent;
  final ValueChanged<bool>? onRecordingChanged;
  final List<({String label, String symbol, VoidCallback? onPressed})>
  creationActions;

  @override
  Widget build(BuildContext context) {
    final menuOpen = useState(false);
    final overlay = useState(false);
    Widget hideDuringVoice(Widget child) => ExcludeSemantics(
      excluding: overlay.value,
      child: IgnorePointer(
        ignoring: overlay.value,
        child: Opacity(opacity: overlay.value ? 0 : 1, child: child),
      ),
    );
    return AgentNavigationOverlay(
      active: overlay,
      onRecordingChanged: onRecordingChanged,
      child: Builder(
        builder: (context) {
          if (defaultTargetPlatform == TargetPlatform.iOS) {
            return SizedBox(
              width: 256,
              height: 56,
              child: Stack(
                clipBehavior: Clip.none,
                children: [
                  hideDuringVoice(
                    IosMessageCreateButton(
                      mergingNavigation: true,
                      onMenuVisibilityChanged: (visible) =>
                          menuOpen.value = visible,
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

  Widget _search(BuildContext context) => SizedBox.square(
    dimension: 56,
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
    required this.menuOpen,
    required this.alignment,
    required this.child,
  });
  final double offset;
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
          duration: duration,
          curve: Curves.easeOutCubic,
          transformAlignment: alignment,
          transform: Matrix4.translationValues(offset, 0, 0)
            ..scaleByDouble(scale, scale, 1, 1),
          child: child,
        ),
      ),
    );
  }
}
