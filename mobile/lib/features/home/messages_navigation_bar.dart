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
    this.axis = Axis.horizontal,
  });
  final Axis axis;
  final VoidCallback onSearch;
  final Widget agent;
  final List<({String label, String symbol, VoidCallback? onPressed})>
  creationActions;

  @override
  Widget build(BuildContext context) => Flex(
    direction: axis,
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
      SizedBox(
        width: axis == Axis.horizontal ? Grid.xs : 0,
        height: axis == Axis.vertical ? Grid.xs : 0,
      ),
      if (defaultTargetPlatform == TargetPlatform.iOS)
        IosMessageCreateButton(
          actions: creationActions,
          label: axis == Axis.horizontal ? 'New' : null,
          showIconWithLabel: axis == Axis.horizontal,
          accessibilityLabel: 'New',
          width: axis == Axis.horizontal ? 112 : 56,
          height: 56,
          plain: false,
        )
      else
        AndroidExpandingMenuButton(actions: creationActions),
      SizedBox(
        width: axis == Axis.horizontal ? Grid.xs : 0,
        height: axis == Axis.vertical ? Grid.xs : 0,
      ),
      agent,
    ],
  );
}
