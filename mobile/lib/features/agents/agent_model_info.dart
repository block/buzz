import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// Read-only model details for agents without mobile model editing.
class AgentModelInfo extends StatelessWidget {
  const AgentModelInfo({
    required this.model,
    this.managementLabel = 'Manage on desktop',
    super.key,
  });
  final String? model;
  final String managementLabel;

  @override
  Widget build(BuildContext context) => Column(
    mainAxisSize: MainAxisSize.min,
    children: [
      if (model != null && model!.trim().isNotEmpty) ...[
        Text(
          model!,
          textAlign: TextAlign.center,
          style: context.textTheme.labelMedium,
        ),
        const SizedBox(height: Grid.half),
      ],
      Text(
        managementLabel,
        textAlign: TextAlign.center,
        style: context.textTheme.bodySmall?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
    ],
  );
}
