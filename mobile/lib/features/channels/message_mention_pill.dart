import 'package:flutter/material.dart';
import '../../shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';

/// The inline identity pill shared by chat mentions and channel actions.
class MessageMentionPill extends StatelessWidget {
  /// Visible contextual identity name.
  final String label;

  /// Full identity label for assistive technologies.
  final String? semanticsLabel;

  /// Whether to use the bot icon instead of the @ prefix.
  final bool isAgent;

  /// Typography inherited from the surrounding message.
  final TextStyle? textStyle;

  /// Creates a chat mention with an agent icon or human @ prefix.
  const MessageMentionPill({
    super.key,
    required this.label,
    this.semanticsLabel,
    required this.isAgent,
    this.textStyle,
  });

  @override
  Widget build(BuildContext context) {
    final style =
        (textStyle ?? context.textTheme.bodyMedium)?.copyWith(
          color: context.colors.primary,
          fontWeight: FontWeight.w500,
          height: 1,
        ) ??
        TextStyle(
          color: context.colors.primary,
          fontWeight: FontWeight.w500,
          height: 1,
        );
    final fontSize = style.fontSize ?? 16;

    return Container(
      padding: const EdgeInsets.fromLTRB(
        Grid.half,
        Grid.quarter + 1,
        Grid.half,
        Grid.quarter,
      ),
      decoration: BoxDecoration(
        color: context.colors.primary.withValues(alpha: 0.1),
        borderRadius: BorderRadius.circular(Radii.sm),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          if (isAgent) ...[
            Icon(
              BuzzIcons.bot,
              size: fontSize * 0.95,
              color: context.colors.primary,
            ),
            const SizedBox(width: Grid.quarter + 1),
          ] else
            Transform.translate(
              offset: const Offset(0, -Grid.quarter),
              child: Text('@', style: style),
            ),
          Flexible(
            child: Text(label, style: style, semanticsLabel: semanticsLabel),
          ),
        ],
      ),
    );
  }
}
