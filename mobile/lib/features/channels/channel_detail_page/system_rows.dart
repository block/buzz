part of '../channel_detail_page.dart';

class _SystemMessageRow extends ConsumerWidget {
  final TimelineMessage message;
  final String channelId;
  final String? currentPubkey;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;

  const _SystemMessageRow({
    required this.message,
    required this.channelId,
    this.currentPubkey,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final systemEvent = message.systemEvent;
    if (systemEvent == null) return const SizedBox.shrink();

    final userCache = ref.watch(userCacheProvider);

    String resolveLabel(String? pubkey) {
      if (pubkey == null) return 'Someone';
      final profile =
          userCache[pubkey.toLowerCase()] ??
          ref.read(userCacheProvider.notifier).get(pubkey.toLowerCase());
      return profile?.label ?? shortPubkey(pubkey);
    }

    final description = systemEvent.describe(resolveLabel);

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onLongPress: () => showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: channelId,
        canManageMessage: false,
        allMessages: null,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                _systemEventAvatar(context, systemEvent, userCache),
                const SizedBox(width: Grid.xxs),
                Expanded(
                  child: Text(
                    description,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
                Text(
                  formatMessageTime(message.createdAt),
                  style: context.textTheme.labelSmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
            ),
            if (message.reactions.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(left: 28),
                child: ReactionRow(
                  reactions: message.reactions,
                  onToggle: (emoji) => toggleReaction(ref, message, emoji),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

Widget _systemEventAvatar(
  BuildContext context,
  SystemEvent event,
  Map<String, UserProfile> userCache,
) {
  final hasTarget =
      event.targetPubkey != null && event.targetPubkey != event.actorPubkey;

  if (event.actorPubkey != null && hasTarget) {
    // Two-avatar stack: actor + target (e.g. "Alice added Bob").
    return SizedBox(
      width: 32,
      height: 20,
      child: Stack(
        children: [
          SmallAvatar(pubkey: event.actorPubkey!, userCache: userCache),
          Positioned(
            left: 12,
            child: SmallAvatar(
              pubkey: event.targetPubkey!,
              userCache: userCache,
            ),
          ),
        ],
      ),
    );
  }

  if (event.actorPubkey != null) {
    return SmallAvatar(pubkey: event.actorPubkey!, userCache: userCache);
  }

  // Fallback: generic icon when no actor is available.
  return Container(
    width: 20,
    height: 20,
    decoration: BoxDecoration(
      color: context.colors.surfaceContainerHighest,
      shape: BoxShape.circle,
    ),
    child: Icon(
      LucideIcons.arrowLeftRight,
      size: 12,
      color: context.colors.onSurfaceVariant,
    ),
  );
}

class _ThreadSummaryRow extends ConsumerWidget {
  final ThreadSummary summary;
  final TimelineMessage message;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _ThreadSummaryRow({
    required this.summary,
    required this.message,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);

    return GestureDetector(
      onTap: () {
        Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: message,
              allMessages: allMessages,
              channelId: channelId,
              currentPubkey: currentPubkey,
              isMember: isMember,
              isArchived: isArchived,
            ),
          ),
        );
      },
      child: Padding(
        padding: const EdgeInsets.only(
          left: 36,
          top: Grid.half,
          bottom: Grid.half,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Stacked participant avatars.
            SizedBox(
              width: 20.0 + (summary.participantPubkeys.length - 1) * 12.0,
              height: 20,
              child: Stack(
                children: [
                  for (var i = 0; i < summary.participantPubkeys.length; i++)
                    Positioned(
                      left: i * 12.0,
                      child: SmallAvatar(
                        pubkey: summary.participantPubkeys[i],
                        userCache: userCache,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            Text(
              '${summary.replyCount} ${summary.replyCount == 1 ? 'reply' : 'replies'}',
              style: context.textTheme.labelMedium?.copyWith(
                color: context.colors.primary,
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(width: Grid.half),
            Icon(
              LucideIcons.chevronRight,
              size: 14,
              color: context.colors.primary,
            ),
          ],
        ),
      ),
    );
  }
}
