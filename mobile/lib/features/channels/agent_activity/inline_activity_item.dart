import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import '../../../shared/theme/theme.dart';
import '../message_content/markdown_escapes.dart';
import 'observer_models.dart';

/// Conversation typography for live updates; raw diagnostics stay in the sheet.
class InlineActivityItem extends StatelessWidget {
  const InlineActivityItem({super.key, required this.item});
  final TranscriptItem item;

  @override
  Widget build(BuildContext context) {
    final text = switch (item) {
      MessageItem i => i.text,
      ThoughtItem i => i.text,
      LifecycleItem i => i.text.isEmpty ? i.title : i.text,
      ToolItem i => '${i.title}${i.isError ? ' · Failed' : ''}',
      MetadataItem _ => '',
    };
    if (text.trim().isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(bottom: Grid.twelve),
      child: GptMarkdown(
        escapeMarkdownLiterals(text),
        style: context.textTheme.bodyMedium?.copyWith(
          color: item is ThoughtItem || item is ToolItem
              ? context.colors.onSurfaceVariant
              : context.colors.onSurface,
        ),
        autolink: false,
      ),
    );
  }
}
