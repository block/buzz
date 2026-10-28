import '../../shared/widgets/android_expanding_menu_button.dart';
import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../../shared/widgets/agent_navigation_overlay.dart';

import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/navigation_button_surface.dart';
import '../../shared/widgets/ios_message_create_button.dart';

/// Separate glass shortcuts for search and creation.
class MessagesNavigationBar extends HookWidget {
  const MessagesNavigationBar({
    super.key,
    required this.onSearch,
    required this.creationActions,
    required this.agent,
  });
  final VoidCallback onSearch;
  final Widget agent;
  final List<({String label, String symbol, VoidCallback? onPressed})>
  creationActions;

  @override
  Widget build(BuildContext context) {
    final overlay = useState(false);
    return AgentNavigationOverlay(
      active: overlay,
      child: SizedBox(
        width: 224 + Grid.xs * 2,
        height: 56,
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            ExcludeSemantics(
              excluding: overlay.value,
              child: IgnorePointer(
                ignoring: overlay.value,
                child: Opacity(
                  opacity: overlay.value ? 0 : 1,
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      NavigationButtonSurface(
                         child: SizedBox.square(
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
                        ),
                      ),
                      const SizedBox(width: Grid.xs),
                      if (defaultTargetPlatform == TargetPlatform.iOS)
                        IosMessageCreateButton(
                          actions: creationActions,
                          label: 'New',
                          showIconWithLabel: true,
                          accessibilityLabel: 'New',
                          width: 112,
                          height: 56,
                          plain: false,
                        )
                      else
                        AndroidExpandingMenuButton(actions: creationActions),
                       const SizedBox(width: Grid.xs),
                      const SizedBox.square(dimension: 56),
                    ],
                  ),
                ),
              ),
            ),
            Positioned(right: 0, top: 0, child: agent),
          ],
        ),
      ),
    );
  }
}
