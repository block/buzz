import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../../shared/theme/theme.dart';
import 'observer_models.dart';

/// Renders a single [TranscriptItem] in the agent activity transcript.
class TranscriptItemWidget extends StatelessWidget {
  final TranscriptItem item;

  const TranscriptItemWidget({super.key, required this.item});

  @override
  Widget build(BuildContext context) {
    return switch (item) {
      final MessageItem i => _MessageItemWidget(item: i),
      final ThoughtItem i => _ThoughtItemWidget(item: i),
      final LifecycleItem i => _LifecycleItemWidget(item: i),
      final MetadataItem i => _MetadataItemWidget(item: i),
      final ToolItem i => _ToolItemWidget(item: i),
    };
  }
}

class _MessageItemWidget extends StatelessWidget {
  final MessageItem item;

  const _MessageItemWidget({required this.item});

  @override
  Widget build(BuildContext context) {
    final isAssistant = item.role == 'assistant';
    final badgeColor = isAssistant
        ? context.colors.primary
        : context.colors.onSurfaceVariant;
    final badgeLabel = isAssistant ? 'Assistant' : 'User';

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.xxs,
                  vertical: Grid.quarter,
                ),
                decoration: BoxDecoration(
                  color: badgeColor.withValues(alpha: 0.12),
                  borderRadius: BorderRadius.circular(8),
                ),
                child: Text(
                  badgeLabel,
                  style: context.textTheme.labelSmall?.copyWith(
                    color: badgeColor,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: Grid.half),
          if (item.text.isNotEmpty)
            GptMarkdown(
              item.text,
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurface,
              ),
            ),
        ],
      ),
    );
  }
}

class _ThoughtItemWidget extends HookWidget {
  final ThoughtItem item;

  const _ThoughtItemWidget({required this.item});

  @override
  Widget build(BuildContext context) {
    final expanded = useState(item.text.length <= 200);

    useEffect(() {
      expanded.value = item.text.length <= 200;
      return null;
    }, [item.id]);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: GestureDetector(
        onTap: () => expanded.value = !expanded.value,
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.all(Grid.xxs),
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest.withValues(
              alpha: 0.5,
            ),
            borderRadius: BorderRadius.circular(8),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Icon(
                    LucideIcons.brain,
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  ),
                  const SizedBox(width: Grid.half),
                  Expanded(
                    child: Text(
                      item.title,
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w600,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  Icon(
                    expanded.value
                        ? LucideIcons.chevronUp
                        : LucideIcons.chevronDown,
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  ),
                ],
              ),
              if (expanded.value && item.text.isNotEmpty) ...[
                const SizedBox(height: Grid.half),
                GptMarkdown(
                  item.text,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _LifecycleItemWidget extends StatelessWidget {
  final LifecycleItem item;

  const _LifecycleItemWidget({required this.item});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: Center(
        child: Text(
          '${item.title}${item.text.isNotEmpty ? ' \u2014 ${item.text}' : ''}',
          style: context.textTheme.labelSmall?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
          textAlign: TextAlign.center,
        ),
      ),
    );
  }
}

class _MetadataItemWidget extends HookWidget {
  final MetadataItem item;

  const _MetadataItemWidget({required this.item});

  @override
  Widget build(BuildContext context) {
    final expanded = useState(false);

    useEffect(() {
      expanded.value = false;
      return null;
    }, [item.id]);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: GestureDetector(
        onTap: () => expanded.value = !expanded.value,
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.all(Grid.xxs),
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest.withValues(
              alpha: 0.3,
            ),
            borderRadius: BorderRadius.circular(8),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Icon(
                    LucideIcons.fileText,
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  ),
                  const SizedBox(width: Grid.half),
                  Expanded(
                    child: Text(
                      '${item.title} (${item.sections.length} sections)',
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w600,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  Icon(
                    expanded.value
                        ? LucideIcons.chevronUp
                        : LucideIcons.chevronDown,
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  ),
                ],
              ),
              if (expanded.value)
                for (final section in item.sections) ...[
                  const SizedBox(height: Grid.xxs),
                  Text(
                    section.title,
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  if (section.body.isNotEmpty) ...[
                    const SizedBox(height: Grid.quarter),
                    Text(
                      section.body.length > 500
                          ? '${section.body.substring(0, 500)}\u2026'
                          : section.body,
                      style: context.textTheme.bodySmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ],
            ],
          ),
        ),
      ),
    );
  }
}

class _ToolItemWidget extends HookWidget {
  final ToolItem item;

  const _ToolItemWidget({required this.item});

  @override
  Widget build(BuildContext context) {
    final argsExpanded = useState(false);
    final resultExpanded = useState(false);

    useEffect(() {
      argsExpanded.value = false;
      resultExpanded.value = false;
      return null;
    }, [item.id]);

    final (statusColor, statusLabel, statusIcon) = _toolStatusDisplay(
      item.status,
      item.isError,
      context,
    );

    final displayName = _formatToolName(item.toolName);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.all(Grid.xxs),
        decoration: BoxDecoration(
          color: context.colors.surfaceContainerHighest.withValues(alpha: 0.4),
          borderRadius: BorderRadius.circular(8),
          border: item.isError
              ? Border.all(color: context.colors.error.withValues(alpha: 0.3))
              : null,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Header: tool name + status badge
            Row(
              children: [
                Icon(LucideIcons.wrench, size: 14, color: statusColor),
                const SizedBox(width: Grid.half),
                Expanded(
                  child: Text(
                    displayName,
                    style: context.textTheme.labelMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.xxs,
                    vertical: Grid.quarter,
                  ),
                  decoration: BoxDecoration(
                    color: statusColor.withValues(alpha: 0.12),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(statusIcon, size: 10, color: statusColor),
                      const SizedBox(width: Grid.quarter),
                      Text(
                        statusLabel,
                        style: context.textTheme.labelSmall?.copyWith(
                          color: statusColor,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
            // Args section
            if (item.args.isNotEmpty) ...[
              const SizedBox(height: Grid.half),
              GestureDetector(
                onTap: () => argsExpanded.value = !argsExpanded.value,
                child: Row(
                  children: [
                    Text(
                      'Arguments',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                    const SizedBox(width: Grid.half),
                    Icon(
                      argsExpanded.value
                          ? LucideIcons.chevronUp
                          : LucideIcons.chevronDown,
                      size: 12,
                      color: context.colors.onSurfaceVariant,
                    ),
                  ],
                ),
              ),
              if (argsExpanded.value) ...[
                const SizedBox(height: Grid.quarter),
                _CodeBlock(text: _prettyJson(item.args)),
              ],
            ],
            // Result section
            if (item.result.isNotEmpty) ...[
              const SizedBox(height: Grid.half),
              GestureDetector(
                onTap: () => resultExpanded.value = !resultExpanded.value,
                child: Row(
                  children: [
                    Text(
                      'Result',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: item.isError
                            ? context.colors.error
                            : context.colors.onSurfaceVariant,
                      ),
                    ),
                    const SizedBox(width: Grid.half),
                    Icon(
                      resultExpanded.value
                          ? LucideIcons.chevronUp
                          : LucideIcons.chevronDown,
                      size: 12,
                      color: item.isError
                          ? context.colors.error
                          : context.colors.onSurfaceVariant,
                    ),
                  ],
                ),
              ),
              if (resultExpanded.value) ...[
                const SizedBox(height: Grid.quarter),
                _CodeBlock(
                  text: item.result.length > 2000
                      ? '${item.result.substring(0, 2000)}\n\n\u2026 (truncated)'
                      : item.result,
                  isError: item.isError,
                ),
              ],
            ],
          ],
        ),
      ),
    );
  }
}

(Color, String, IconData) _toolStatusDisplay(
  ToolStatus status,
  bool isError,
  BuildContext context,
) {
  if (isError || status == ToolStatus.failed) {
    return (context.colors.error, 'Error', LucideIcons.circleX);
  }
  if (status == ToolStatus.completed) {
    return (context.appColors.success, 'Done', LucideIcons.circleCheck);
  }
  if (status == ToolStatus.pending) {
    return (context.colors.onSurfaceVariant, 'Pending', LucideIcons.circleDot);
  }
  return (context.appColors.warning, 'Running', LucideIcons.clock3);
}

String _formatToolName(String toolName) {
  return toolName
      .split('_')
      .map(
        (part) =>
            part.isEmpty ? '' : '${part[0].toUpperCase()}${part.substring(1)}',
      )
      .join(' ');
}

String _prettyJson(Map<String, dynamic> value) {
  try {
    return const JsonEncoder.withIndent('  ').convert(value);
  } catch (_) {
    return value.toString();
  }
}

class _CodeBlock extends StatelessWidget {
  final String text;
  final bool isError;

  const _CodeBlock({required this.text, this.isError = false});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(Grid.xxs),
      decoration: BoxDecoration(
        color: isError
            ? context.colors.error.withValues(alpha: 0.06)
            : context.colors.surface,
        borderRadius: BorderRadius.circular(6),
      ),
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Text(
          text,
          softWrap: false,
          style: context.textTheme.bodySmall?.copyWith(
            fontFamily: 'monospace',
            fontSize: 11,
            color: isError
                ? context.colors.error
                : context.colors.onSurfaceVariant,
          ),
        ),
      ),
    );
  }
}
