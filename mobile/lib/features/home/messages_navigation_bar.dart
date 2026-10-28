import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/concentric_sheet_surface.dart';
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
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      ConcentricSheetSurface(
        enabled: true,
        usesGlass: true,
        providesSheetSurface: false,
        padding: EdgeInsets.zero,
        minimumRadius: Radii.navigationCapsule,
        contentClipRadius: Radii.navigationCapsule,
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
        ConcentricSheetSurface(
          enabled: true,
          usesGlass: true,
          providesSheetSurface: false,
          padding: EdgeInsets.zero,
          minimumRadius: Radii.navigationCapsule,
          contentClipRadius: Radii.navigationCapsule,
          child: PopupMenuButton<int>(
            tooltip: 'New',
            onOpened: () => unawaited(HapticFeedback.lightImpact()),
            onSelected: (index) => creationActions[index].onPressed?.call(),
            itemBuilder: (_) => [
              for (var i = 0; i < creationActions.length; i++)
                PopupMenuItem(
                  value: i,
                  enabled: creationActions[i].onPressed != null,
                  child: Text(creationActions[i].label),
                ),
            ],
            child: const SizedBox(
              width: 112,
              height: 56,
              child: Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(BuzzIcons.plus, size: 24),
                  SizedBox(width: Grid.xxs),
                  Text('New'),
                ],
              ),
            ),
          ),
        ),
      const SizedBox(width: Grid.xs),
      agent,
    ],
  );
}
