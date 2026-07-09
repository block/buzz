part of '../channel_detail_page.dart';

class _MessageBubble extends ConsumerWidget {
  final TimelineMessage message;
  final bool showAuthor;
  final Map<String, String> channelNames;
  final String currentChannelId;
  final String? currentPubkey;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;

  const _MessageBubble({
    required this.message,
    required this.showAuthor,
    required this.channelNames,
    required this.currentChannelId,
    required this.currentPubkey,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Watch only this user's profile to avoid rebuilding on unrelated cache changes.
    final pk = message.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? shortPubkey(message.pubkey);

    // Build mention names map from event p-tags.
    final userCache = ref.watch(userCacheProvider);
    final mentionNames = <String, String>{};
    for (final mpk in message.mentionPubkeys) {
      final p = userCache[mpk.toLowerCase()];
      if (p?.displayName != null) {
        mentionNames[mpk.toLowerCase()] = p!.displayName!;
      }
    }

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onLongPress: () => showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: currentChannelId,
        canManageMessage:
            currentPubkey?.toLowerCase() == pk ||
            (profile?.ownerPubkey != null &&
                profile?.ownerPubkey == currentPubkey?.toLowerCase()),
        allMessages: allMessages,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
      ),
      child: Padding(
        padding: EdgeInsets.only(top: showAuthor ? Grid.xs : Grid.half),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (showAuthor)
              GestureDetector(
                onTap: () => showUserProfileSheet(context, message.pubkey),
                child: _UserAvatar(profile: profile, pubkey: message.pubkey),
              )
            else
              const SizedBox(width: 28),
            const SizedBox(width: Grid.xxs),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (showAuthor)
                    Padding(
                      padding: const EdgeInsets.only(bottom: Grid.quarter),
                      child: Row(
                        children: [
                          GestureDetector(
                            onTap: () =>
                                showUserProfileSheet(context, message.pubkey),
                            child: Text(
                              displayName,
                              style: context.textTheme.labelMedium?.copyWith(
                                fontWeight: FontWeight.w600,
                                color: context.colors.onSurface,
                              ),
                            ),
                          ),
                          const SizedBox(width: Grid.xxs),
                          Text(
                            formatMessageTime(message.createdAt),
                            style: context.textTheme.labelSmall?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                          if (message.edited) ...[
                            const SizedBox(width: Grid.half),
                            Text(
                              '(edited)',
                              style: context.textTheme.labelSmall?.copyWith(
                                color: context.colors.onSurfaceVariant,
                                fontStyle: FontStyle.italic,
                              ),
                            ),
                          ],
                        ],
                      ),
                    ),
                  MessageContent(
                    content: message.content,
                    mentionNames: mentionNames,
                    channelNames: channelNames,
                    tags: message.tags,
                    onChannelTap: (channelId) {
                      openChannelLink(
                        context: context,
                        ref: ref,
                        channelId: channelId,
                        currentChannelId: currentChannelId,
                      );
                    },
                  ),
                  if (message.reactions.isNotEmpty)
                    ReactionRow(
                      reactions: message.reactions,
                      onToggle: (emoji) => toggleReaction(ref, message, emoji),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _UserAvatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;

  const _UserAvatar({required this.profile, required this.pubkey});

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return CircleAvatar(
      radius: 14,
      backgroundColor: context.colors.primaryContainer,
      backgroundImage: avatarUrl != null ? NetworkImage(avatarUrl) : null,
      child: avatarUrl == null
          ? Text(
              initial,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onPrimaryContainer,
                fontWeight: FontWeight.w600,
              ),
            )
          : null,
    );
  }
}

IconData channelIcon(Channel channel) {
  if (channel.isDm) return LucideIcons.messagesSquare;
  if (channel.isPrivate) return LucideIcons.lock;
  if (channel.isForum) return LucideIcons.messageSquareText;
  return LucideIcons.hash;
}
