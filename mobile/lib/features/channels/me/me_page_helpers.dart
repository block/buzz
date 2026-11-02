part of 'me_page.dart';

class _MeRetry extends StatelessWidget {
  final VoidCallback onRetry;
  const _MeRetry({required this.onRetry});
  @override
  Widget build(BuildContext context) => Center(
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        const Text('Couldn’t load your chats and projects.'),
        TextButton(onPressed: onRetry, child: const Text('Try again')),
      ],
    ),
  );
}

class _MeChatRow extends ConsumerWidget {
  const _MeChatRow({
    required this.channelId,
    required this.name,
    required this.onTap,
    required this.isUnread,
  });
  final String channelId;
  final String name;
  final bool isUnread;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final color = channelNavigationForeground(context, unread: isUnread);
    final working = ref.watch(meWorkingAgentsProvider(channelId));
    final status = working.isEmpty ? '' : ', ${working.join(', ')} working';
    return Semantics(
      label: '${isUnread ? '$name, unread' : name}$status',
      onTap: onTap,
      excludeSemantics: true,
      button: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.xxs,
            vertical: Grid.inventoryRowInset,
          ),
          child: Row(
            children: [
              SizedBox(
                width: 24,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Icon(BuzzIcons.messageCircle, size: 20, color: color),
                ),
              ),
              const SizedBox(width: Grid.inventoryLabelGap),
              Expanded(
                child: Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: navigationChatTitleTextStyle.copyWith(
                    color: color,
                    fontWeight: isUnread ? FontWeight.w600 : FontWeight.w400,
                  ),
                ),
              ),
              if (working.isNotEmpty) ...[
                const SizedBox(width: Grid.xs),
                SoloWorkingDots(color: color),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

// Reconnects replace the transport service while an editor may still be open.
// Retain the reviewed account/community boundary but use its current connection.
MeSync _currentMeSync(WidgetRef ref, MeSync reviewed) {
  final current = ref.read(meSyncProvider);
  if (current.community != reviewed.community ||
      current.pubkey != reviewed.pubkey) {
    throw StateError('Your account or community changed. Open Solo again.');
  }
  return current;
}

class _MeSectionEmpty extends StatelessWidget {
  const _MeSectionEmpty(this.label);
  final String label;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(
      horizontal: Grid.xxs,
      vertical: Grid.half,
    ),
    child: Align(
      alignment: Alignment.centerLeft,
      child: Text(
        label,
        style: contentListBodyTextStyle.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
    ),
  );
}
