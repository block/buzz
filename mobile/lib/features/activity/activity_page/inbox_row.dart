part of '../activity_page.dart';

/// "New" boundary between unread and previously read rows.
class _NewBoundaryDivider extends StatelessWidget {
  const _NewBoundaryDivider();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.half,
      ),
      child: Row(
        children: [
          Expanded(
            child: Divider(color: context.colors.error.withValues(alpha: 0.5)),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
            child: Text(
              'New',
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.error,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          Expanded(
            child: Divider(color: context.colors.error.withValues(alpha: 0.5)),
          ),
        ],
      ),
    );
  }
}

/// One conversation row, matching desktop's inbox item hierarchy:
/// avatar | sender + unread dot + time | contextual label | preview.
class _InboxRow extends ConsumerWidget {
  final InboxItem item;
  final Channel? channel;
  final String? currentPubkey;
  final bool isDone;
  final VoidCallback onTap;
  final VoidCallback onMarkRead;
  final VoidCallback onMarkUnread;

  const _InboxRow({
    required this.item,
    required this.channel,
    required this.currentPubkey,
    required this.isDone,
    required this.onTap,
    required this.onMarkRead,
    required this.onMarkUnread,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);
    final profile = userCache[item.item.pubkey.toLowerCase()];
    final senderLabel = profile?.displayName ?? shortPubkey(item.item.pubkey);

    final isDm = channel?.isDm ?? false;
    final channelName = channel != null && !isDm
        ? (channel!.name.isNotEmpty ? channel!.name : null)
        : null;
    final label = inboxTypeLabel(
      item,
      channelName: channelName,
      isDm: isDm,
      senderLabel: senderLabel,
    );

    final mutedColor = context.colors.onSurfaceVariant;
    final labelColor = item.isActionRequired && !isDone
        ? context.colors.tertiary
        : mutedColor;

    return InkWell(
      key: ValueKey('inbox-row-${item.id}'),
      onTap: onTap,
      onLongPress: () => _showRowActions(context),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.gutter,
          vertical: Grid.twelve,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            _RowAvatar(pubkey: item.item.pubkey, profile: profile),
            const SizedBox(width: Grid.twelve),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  // Sender + unread dot + timestamp.
                  Row(
                    children: [
                      Expanded(
                        child: Text(
                          senderLabel,
                          // Compact label scale — matches the old
                          // "@ Mention" headline treatment while staying
                          // the row's primary label.
                          style: context.textTheme.labelMedium?.copyWith(
                            fontWeight: FontWeight.w600,
                          ),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      const SizedBox(width: Grid.xxs),
                      if (!isDone) ...[
                        Container(
                          key: ValueKey('inbox-unread-dot-${item.id}'),
                          width: 6,
                          height: 6,
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: context.colors.primary,
                          ),
                        ),
                        const SizedBox(width: Grid.half),
                      ],
                      Text(
                        _inboxTimestamp(item.latestActivityAt),
                        style: context.textTheme.labelSmall?.copyWith(
                          color: mutedColor,
                          fontWeight: isDone
                              ? FontWeight.w400
                              : FontWeight.w500,
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: Grid.quarter),
                  // Contextual label: "Mentioned in #channel" etc.
                  Row(
                    children: [
                      Flexible(
                        child: Text(
                          label.text,
                          style: context.textTheme.labelSmall?.copyWith(
                            color: labelColor,
                            fontWeight: FontWeight.w500,
                          ),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      if (label.channelLabel != null) ...[
                        const SizedBox(width: Grid.half),
                        Flexible(
                          child: Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: Grid.half + Grid.quarter,
                              vertical: Grid.quarter / 2,
                            ),
                            decoration: BoxDecoration(
                              color: context.colors.surfaceContainerHighest,
                              borderRadius: BorderRadius.circular(Grid.half),
                            ),
                            child: Text(
                              '#${label.channelLabel}',
                              style: context.textTheme.labelSmall?.copyWith(
                                color: mutedColor,
                              ),
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                        ),
                      ],
                    ],
                  ),
                  const SizedBox(height: Grid.half),
                  // Preview (bold while unread — desktop parity).
                  MessageContent(
                    content: item.item.displayContent,
                    tags: item.item.tags,
                    maxLines: 2,
                    baseStyle: context.textTheme.bodySmall?.copyWith(
                      color: isDone ? mutedColor : context.colors.onSurface,
                      fontWeight: isDone ? FontWeight.w400 : FontWeight.w600,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  void _showRowActions(BuildContext context) {
    showModalBottomSheet<void>(
      context: context,
      builder: (sheetContext) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: Icon(
                isDone ? LucideIcons.mail : LucideIcons.mailOpen,
                size: 20,
              ),
              title: Text(isDone ? 'Mark unread' : 'Mark as read'),
              onTap: () {
                Navigator.of(sheetContext).pop();
                isDone ? onMarkUnread() : onMarkRead();
              },
            ),
            ListTile(
              leading: const Icon(LucideIcons.externalLink, size: 20),
              title: const Text('Open conversation'),
              onTap: () {
                Navigator.of(sheetContext).pop();
                onTap();
              },
            ),
          ],
        ),
      ),
    );
  }
}

class _RowAvatar extends StatelessWidget {
  final String pubkey;
  final UserProfile? profile;

  const _RowAvatar({required this.pubkey, required this.profile});

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    return AvatarImage(
      imageUrl: profile?.avatarUrl,
      radius: 18,
      backgroundColor: context.colors.primaryContainer,
      fallback: Text(
        initial,
        style: TextStyle(
          fontSize: 14,
          fontWeight: FontWeight.w600,
          color: context.colors.onPrimaryContainer,
        ),
      ),
    );
  }
}
