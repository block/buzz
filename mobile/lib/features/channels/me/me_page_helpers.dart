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

class _MeChatRow extends HookConsumerWidget {
  const _MeChatRow({
    required this.channelId,
    required this.name,
    required this.onTap,
    required this.onLongPress,
    required this.isUnread,
    required this.canArchive,
  });
  final String channelId;
  final String name;
  final bool isUnread;
  final bool canArchive;
  final VoidCallback onTap;
  final VoidCallback onLongPress;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final archiving = useState(false);
    Future<void> archive() async {
      if (!canArchive || archiving.value) return;
      archiving.value = true;
      try {
        await ref.read(channelActionsProvider).archiveChannel(channelId);
      } catch (error) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            SnackBar(content: Text('Couldn’t archive chat: $error')),
            kind: ToastKind.error,
          );
        }
      } finally {
        if (context.mounted) archiving.value = false;
      }
    }

    final color = channelNavigationForeground(context, unread: isUnread);
    final working = ref.watch(meWorkingAgentsProvider(channelId));
    final status = archiving.value
        ? ', archiving'
        : working.isEmpty
        ? ''
        : ', ${working.join(', ')} working';
    final row = Semantics(
      label: '${isUnread ? '$name, unread' : name}$status',
      liveRegion: archiving.value,
      onTap: onTap,
      onLongPress: onLongPress,
      customSemanticsActions: {
        if (canArchive && !archiving.value)
          const CustomSemanticsAction(label: 'Archive chat'): archive,
      },
      excludeSemantics: true,
      button: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: onTap,
        onLongPress: onLongPress,
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
              if (archiving.value) ...[
                const SizedBox(width: Grid.xs),
                const BuzzLoadingIndicator(size: 20),
              ] else if (working.isNotEmpty) ...[
                const SizedBox(width: Grid.xs),
                SoloWorkingDots(color: color),
              ],
            ],
          ),
        ),
      ),
    );
    if (!canArchive) return row;
    return Dismissible(
      key: ValueKey('archive-solo-$channelId'),
      // Keep the axis stable while confirmDismiss is pending. Dismissible
      // blocks further drags then; archive() also guards semantic requests.
      direction: DismissDirection.endToStart,
      movementDuration: MediaQuery.disableAnimationsOf(context)
          ? Duration.zero
          : const Duration(milliseconds: 200),
      // The channel list owns removal, only after the relay accepts the
      // archive. A rejection leaves the chat in place and ready to retry.
      confirmDismiss: (_) async {
        await archive();
        return false;
      },
      background: ExcludeSemantics(
        child: Container(
          alignment: AlignmentDirectional.centerEnd,
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          decoration: BoxDecoration(
            color: context.colors.primaryContainer,
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (archiving.value)
                BuzzLoadingIndicator(
                  size: 20,
                  color: context.colors.onPrimaryContainer,
                )
              else
                Icon(
                  BuzzIcons.archive,
                  color: context.colors.onPrimaryContainer,
                ),
              const SizedBox(width: Grid.xs),
              Text(
                archiving.value ? 'Archiving…' : 'Archive',
                style: TextStyle(color: context.colors.onPrimaryContainer),
              ),
            ],
          ),
        ),
      ),
      child: row,
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
