import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../../shared/theme/theme.dart';
import 'shared_activity_models.dart';

/// Privacy-safe presentation of member-visible managed-agent activity.
class SharedActivitySummary extends StatelessWidget {
  final List<SharedActivity> activities;
  final ScrollController? controller;
  final EdgeInsetsGeometry? padding;

  const SharedActivitySummary({
    super.key,
    required this.activities,
    this.controller,
    this.padding,
  });

  @override
  Widget build(BuildContext context) {
    return ListView.separated(
      controller: controller,
      padding: padding ?? const EdgeInsets.all(Grid.gutter),
      itemCount: activities.length,
      separatorBuilder: (_, _) => const SizedBox(height: Grid.xxs),
      itemBuilder: (context, index) =>
          _ActivityRow(activity: activities[index]),
    );
  }
}

class _ActivityRow extends StatelessWidget {
  final SharedActivity activity;

  const _ActivityRow({required this.activity});

  @override
  Widget build(BuildContext context) {
    final title = _activityTitle(activity);
    final details = _activityDetails(activity);
    return Container(
      padding: const EdgeInsets.all(Grid.sm),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.md),
        border: Border.all(color: context.colors.outlineVariant),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(
            _activityIcon(activity),
            size: 18,
            color: context.colors.onSurfaceVariant,
          ),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: context.textTheme.bodyMedium?.copyWith(
                    fontWeight: FontWeight.w600,
                  ),
                ),
                if (details case final details?) ...[
                  const SizedBox(height: Grid.quarter),
                  Text(
                    details,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ],
                if (activity.usage case final usage?) ...[
                  const SizedBox(height: Grid.half),
                  Text(
                    'Per-turn token usage',
                    style: context.textTheme.labelMedium?.copyWith(
                      color: context.colors.onSurfaceVariant,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: Grid.quarter),
                  Wrap(
                    spacing: Grid.xxs,
                    runSpacing: Grid.quarter,
                    children: _usageLabels(usage)
                        .map(
                          (label) => Text(
                            label,
                            style: context.textTheme.bodySmall?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        )
                        .toList(),
                  ),
                ],
              ],
            ),
          ),
          if (activity.activityClass != SharedActivityClass.usage) ...[
            const SizedBox(width: Grid.xxs),
            Text(
              _statusLabel(activity.status),
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant,
                fontWeight: FontWeight.w600,
              ),
            ),
          ],
        ],
      ),
    );
  }
}

String _activityTitle(SharedActivity activity) =>
    switch (activity.activityClass) {
      SharedActivityClass.turn => 'Working',
      SharedActivityClass.tool => _toolLabel(activity.toolKind!),
      SharedActivityClass.usage => 'Per-turn token usage',
    };

String? _activityDetails(SharedActivity activity) {
  final durationMs = activity.durationMs;
  return durationMs == null ? null : 'Duration: ${_durationLabel(durationMs)}';
}

IconData _activityIcon(SharedActivity activity) =>
    switch (activity.activityClass) {
      SharedActivityClass.turn => LucideIcons.activity,
      SharedActivityClass.tool => LucideIcons.wrench,
      SharedActivityClass.usage => LucideIcons.chartNoAxesColumn,
    };

String _toolLabel(SharedActivityToolKind toolKind) => switch (toolKind) {
  SharedActivityToolKind.read => 'Read',
  SharedActivityToolKind.edit => 'Edit',
  SharedActivityToolKind.delete => 'Delete',
  SharedActivityToolKind.move => 'Move',
  SharedActivityToolKind.search => 'Search',
  SharedActivityToolKind.execute => 'Execute',
  SharedActivityToolKind.think => 'Working',
  SharedActivityToolKind.fetch => 'Fetch',
  SharedActivityToolKind.switchMode => 'Switch mode',
  SharedActivityToolKind.other => 'Tool',
};

String _statusLabel(SharedActivityStatus status) => switch (status) {
  SharedActivityStatus.started ||
  SharedActivityStatus.pending ||
  SharedActivityStatus.running => 'Working',
  SharedActivityStatus.completed => 'Completed',
  SharedActivityStatus.failed => 'Failed',
  SharedActivityStatus.cancelled => 'Cancelled',
};

String _durationLabel(int milliseconds) {
  if (milliseconds < 1000) return '$milliseconds ms';
  final seconds = milliseconds / 1000;
  final value = seconds == seconds.roundToDouble()
      ? seconds.toStringAsFixed(0)
      : seconds.toStringAsFixed(1);
  return '$value s';
}

List<String> _usageLabels(SharedActivityUsage usage) => [
  if (usage.inputTokens case final value?) 'Input: $value',
  if (usage.outputTokens case final value?) 'Output: $value',
  if (usage.totalTokens case final value?) 'Total: $value',
  if (usage.cacheReadTokens case final value?) 'Cache read: $value',
  if (usage.cacheWriteTokens case final value?) 'Cache write: $value',
];
