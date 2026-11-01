import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/navigation_button_surface.dart';
import '../../shared/widgets/android_expanding_menu_button.dart';
import '../../shared/widgets/ios_message_create_button.dart';

/// Separate glass shortcuts for search and creation.
class MessagesNavigationBar extends StatelessWidget {
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
  Widget build(BuildContext context) => Builder(
    builder: (controlsContext) => Row(
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
            dividerBefore: const {2},
            sectionTitles: const {0: 'Solo', 2: 'Team'},
            label: 'New',
            showIconWithLabel: true,
            accessibilityLabel: 'New',
            width: 112,
            height: 56,
            plain: false,
          )
        else
          AndroidExpandingMenuButton(
            expansionBoundsContext: controlsContext,
            actions: creationActions,
            dividerBefore: const {2},
            sectionTitles: const {0: 'Solo', 2: 'Team'},
          ),
        const SizedBox(width: Grid.xs),
        agent,
      ],
    ),
  );
}
