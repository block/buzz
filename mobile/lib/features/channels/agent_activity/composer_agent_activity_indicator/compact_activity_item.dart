part of '../composer_agent_activity_indicator.dart';

class _CompactActivityItem extends StatelessWidget {
  final TranscriptItem item;

  const _CompactActivityItem({required this.item});

  @override
  Widget build(BuildContext context) {
    final (icon, label, color) = switch (item) {
      final ToolItem tool => (
        tool.isError || tool.status == ToolStatus.failed
            ? LucideIcons.circleX
            : tool.status == ToolStatus.completed
            ? LucideIcons.circleCheck
            : LucideIcons.wrench,
        _toolActivityLabel(tool),
        tool.isError || tool.status == ToolStatus.failed
            ? context.colors.error
            : context.colors.onSurfaceVariant,
      ),
      final ThoughtItem thought => (
        LucideIcons.brain,
        thought.title,
        context.colors.onSurfaceVariant,
      ),
      final MessageItem message => (
        LucideIcons.messageSquare,
        '${message.role == 'assistant' ? 'Response' : 'Prompt'}: ${_oneLine(message.text)}',
        context.colors.onSurfaceVariant,
      ),
      final LifecycleItem lifecycle => (
        lifecycle.title.toLowerCase().contains('error')
            ? LucideIcons.circleX
            : LucideIcons.activity,
        '${lifecycle.title}${lifecycle.text.isEmpty ? '' : ': ${_oneLine(lifecycle.text)}'}',
        lifecycle.title.toLowerCase().contains('error')
            ? context.colors.error
            : context.colors.onSurfaceVariant,
      ),
      MetadataItem() => (
        LucideIcons.activity,
        'Activity',
        context.colors.onSurfaceVariant,
      ),
    };
    return Padding(
      key: ValueKey('compact-activity-${item.id}'),
      padding: const EdgeInsets.symmetric(vertical: Grid.quarter),
      child: Row(
        children: [
          Icon(icon, size: 14, color: color),
          const SizedBox(width: Grid.half),
          Expanded(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: context.textTheme.bodySmall?.copyWith(color: color),
            ),
          ),
        ],
      ),
    );
  }
}
