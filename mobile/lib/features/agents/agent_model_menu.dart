import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/android_expanding_menu_button.dart';
import '../../shared/widgets/ios_message_create_button.dart';

/// Shared platform menu for agent model information and available actions.
class AgentModelMenu extends StatelessWidget {
  const AgentModelMenu({required this.label, required this.actions, super.key});
  final String label;
  final List<({String label, String symbol, VoidCallback? onPressed})> actions;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final width = constraints.maxWidth;
      if (defaultTargetPlatform == TargetPlatform.iOS) {
        return IosMessageCreateButton(
          label: label,
          labelFontSize: 13,
          showIconWithLabel: false,
          accessibilityLabel: label,
          plain: false,
          width: width,
          height: 44,
          actions: actions,
        );
      }
      return AndroidExpandingMenuButton(
        label: label,
        width: width,
        height: 44,
        actions: actions,
        trigger: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Flexible(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: context.textTheme.labelMedium?.copyWith(fontSize: 12),
                ),
              ),
            ],
          ),
        ),
      );
    },
  );
}
