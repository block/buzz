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
    final canManageMessage =
        currentPubkey?.toLowerCase() == pk ||
        (profile?.ownerPubkey != null &&
            profile?.ownerPubkey == currentPubkey?.toLowerCase());

    // Build mention names map from event p-tags.
    final userCache = ref.watch(userCacheProvider);
    final knownAgentPubkeys = ref.watch(
      mentionAgentPubkeysProvider(currentChannelId),
    );
    final mentionNames = <String, String>{};
    final agentMentionPubkeys = <String>{};
    for (final mpk in message.mentionPubkeys) {
      final normalizedPubkey = mpk.toLowerCase();
      final p = userCache[normalizedPubkey];
      if (p?.displayName != null) {
        mentionNames[normalizedPubkey] = p!.displayName!;
      }
      if (knownAgentPubkeys.contains(normalizedPubkey)) {
        agentMentionPubkeys.add(normalizedPubkey);
      }
    }

    return Material(
      color: Colors.transparent,
      borderRadius: BorderRadius.circular(Radii.md),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        key: ValueKey('message-row-${message.id}'),
        borderRadius: BorderRadius.circular(Radii.md),
        highlightColor: context.colors.primary.withValues(alpha: 0.1),
        onLongPress: () => showMessageActions(
          context: context,
          ref: ref,
          message: message,
          channelId: currentChannelId,
          canManageMessage: canManageMessage,
          allMessages: allMessages,
          currentPubkey: currentPubkey,
          isMember: isMember,
          isArchived: isArchived,
        ),
        child: Padding(
          padding: EdgeInsets.only(
            top: showAuthor ? Grid.xs : Grid.xxs,
            bottom: showAuthor ? 0 : Grid.xxs,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (showAuthor)
                GestureDetector(
                  onTap: () => showUserProfileSheet(context, message.pubkey),
                  child: _UserAvatar(profile: profile, pubkey: message.pubkey),
                )
              else
                const SizedBox(width: messageAvatarSize),
              const SizedBox(width: messageAvatarContentGap),
              Expanded(
                child: Transform.translate(
                  offset: Offset(0, showAuthor ? -Grid.quarter : 0),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (showAuthor)
                        Padding(
                          padding: const EdgeInsets.only(bottom: Grid.quarter),
                          child: Row(
                            children: [
                              Expanded(
                                child: MessageAuthorMeta(
                                  displayName: displayName,
                                  username: messageUsernameLabel(profile),
                                  timestamp: formatMessageTime(
                                    message.createdAt,
                                  ),
                                  nameColor: context.colors.onSurface,
                                  metadataColor:
                                      context.colors.onSurfaceVariant,
                                  onAuthorTap: () => showUserProfileSheet(
                                    context,
                                    message.pubkey,
                                  ),
                                  displayNameKey: ValueKey(
                                    'message-author-${message.id}',
                                  ),
                                  usernameKey: ValueKey(
                                    'message-username-${message.id}',
                                  ),
                                  timestampKey: ValueKey(
                                    'message-timestamp-${message.id}',
                                  ),
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
                        agentMentionPubkeys: agentMentionPubkeys,
                        channelNames: channelNames,
                        tags: message.tags,
                        baseStyle: messageBodyTextStyle.copyWith(
                          color: context.colors.onSurface,
                        ),
                        mediaCarouselTrailingOverflow: Grid.gutter,
                        onMediaReply: allMessages == null
                            ? null
                            : () {
                                if (!context.mounted) return;
                                Navigator.of(context).push(
                                  MaterialPageRoute<void>(
                                    builder: (_) => ThreadDetailPage(
                                      threadHead: message,
                                      allMessages: allMessages!,
                                      channelId: currentChannelId,
                                      currentPubkey: currentPubkey,
                                      isMember: isMember,
                                      isArchived: isArchived,
                                    ),
                                  ),
                                );
                              },
                        onMediaMore: (viewerContext, imageUrl) =>
                            showImageActions(
                              context: viewerContext,
                              ref: ref,
                              message: message,
                              channelId: currentChannelId,
                              imageUrl: imageUrl,
                              canManageMessage: canManageMessage,
                              onDeleted: () {
                                if (viewerContext.mounted) {
                                  Navigator.of(viewerContext).maybePop();
                                }
                              },
                            ),
                        onChannelTap: (channelId) {
                          openChannelLink(
                            context: context,
                            ref: ref,
                            channelId: channelId,
                            currentChannelId: currentChannelId,
                          );
                        },
                        onMentionTap: (pubkey) =>
                            showUserProfileSheet(context, pubkey),
                      ),
                      if (message.reactions.isNotEmpty)
                        ReactionRow(
                          reactions: message.reactions,
                          onToggle: (emoji) =>
                              toggleReaction(ref, message, emoji),
                        ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

Widget _messageTimestamp(BuildContext context, int createdAt, {Key? key}) {
  return ConstrainedBox(
    constraints: const BoxConstraints(maxWidth: Grid.xxl),
    child: Text(
      key: key,
      formatMessageTime(createdAt),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: messageTimestampTextStyle.copyWith(
        color: context.colors.onSurfaceVariant,
      ),
    ),
  );
}

class _UserAvatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;
  final double size;

  const _UserAvatar({
    required this.profile,
    required this.pubkey,
    this.size = messageAvatarSize,
  });

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return AvatarImage(
      imageUrl: avatarUrl,
      radius: size / 2,
      backgroundColor: context.colors.primaryContainer,
      fallback: Text(
        initial,
        style:
            (size > 28
                    ? context.textTheme.labelMedium
                    : context.textTheme.labelSmall)
                ?.copyWith(
                  color: context.colors.onPrimaryContainer,
                  fontWeight: FontWeight.w600,
                ),
      ),
    );
  }
}

IconData channelIcon(Channel channel) {
  if (channel.isDm) return LucideIcons.messagesSquare;
  if (channel.isPrivate) return LucideIcons.lock;
  if (channel.isForum) return LucideIcons.messageSquareText;
  return LucideIcons.hash;
}
