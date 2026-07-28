import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../profile/user_cache_provider.dart';
import 'channel.dart';
import 'channels_provider.dart';
import 'date_formatters.dart';
import 'message_content.dart';
import 'timeline_message.dart';

/// Quoted rendering of a forwarded message (kind 40009): an attribution row
/// ("Forwarded from #channel" / "a private channel" / "a direct message")
/// followed by a bordered card with the original author, timestamp, and body.
class ForwardedMessageQuote extends ConsumerWidget {
  final ForwardInfo forward;

  const ForwardedMessageQuote({super.key, required this.forward});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final original = forward.original;
    final authorPk = original.pubkey.toLowerCase();
    final authorProfile =
        ref.watch(userCacheProvider.select((cache) => cache[authorPk])) ??
        ref.read(userCacheProvider.notifier).get(authorPk);
    final authorName =
        authorProfile?.label ??
        (original.pubkey.length >= 8
            ? '${original.pubkey.substring(0, 8)}...'
            : original.pubkey);

    final metaStyle = context.textTheme.labelSmall?.copyWith(
      color: context.colors.onSurfaceVariant,
    );

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.only(top: Grid.quarter),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                LucideIcons.forward,
                size: 14,
                color: context.colors.onSurfaceVariant,
              ),
              const SizedBox(width: Grid.quarter),
              Flexible(
                child: Text(
                  _attributionLabel(ref),
                  style: metaStyle,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ),
        Container(
          margin: const EdgeInsets.only(top: Grid.quarter),
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.xxs,
            vertical: Grid.half,
          ),
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest.withValues(
              alpha: 0.4,
            ),
            borderRadius: BorderRadius.circular(Radii.md),
            border: Border.all(color: context.colors.outlineVariant),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(
                children: [
                  Flexible(
                    child: Text(
                      authorName,
                      style: context.textTheme.labelMedium?.copyWith(
                        fontWeight: FontWeight.w600,
                        color: context.colors.onSurface,
                      ),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  const SizedBox(width: Grid.xxs),
                  Text(formatMessageTime(original.createdAt), style: metaStyle),
                ],
              ),
              const SizedBox(height: Grid.quarter),
              MessageContent(content: original.content, tags: original.tags),
            ],
          ),
        ),
      ],
    );
  }

  String _attributionLabel(WidgetRef ref) {
    switch (forward.sourceType) {
      case ForwardSourceType.channel:
        final channels =
            ref.watch(channelsProvider).asData?.value ?? const <Channel>[];
        Channel? source;
        for (final channel in channels) {
          if (channel.id == forward.sourceChannelId) {
            source = channel;
            break;
          }
        }
        return source != null && source.name.isNotEmpty
            ? 'Forwarded from #${source.name}'
            : 'Forwarded from a channel';
      case ForwardSourceType.private:
        return 'Forwarded from a private channel';
      case ForwardSourceType.dm:
        return 'Forwarded from a direct message';
    }
  }
}
