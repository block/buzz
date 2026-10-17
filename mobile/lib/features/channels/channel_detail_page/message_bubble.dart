part of '../channel_detail_page.dart';

class _MessageBubble extends HookConsumerWidget {
  final TimelineMessage message;
  final bool showAuthor;
  final bool typingHandoff;
  final bool showAvatar;
  final bool hasReplies;
  final Widget? replyFooter;
  final Map<String, String> channelNames;
  final String currentChannelId;
  final String? currentPubkey;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;
  final FocusNode? composerFocusNode;
  final VoidCallback? restoreComposerFocus;

  const _MessageBubble({
    required this.message,
    required this.showAuthor,
    this.typingHandoff = false,
    this.showAvatar = true,
    required this.hasReplies,
    this.replyFooter,
    required this.channelNames,
    required this.currentChannelId,
    required this.currentPubkey,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
    this.composerFocusNode,
    this.restoreComposerFocus,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final messageSnapshotKey = useMemoized(GlobalKey.new, const []);
    final mediaOffset = useMessageMediaOffset(message.content, message.tags);
    final hasLocalReplies = ref.watch(
      threadLocalRepliesProvider(
        ThreadRepliesArgs(
          channelId: currentChannelId,
          rootId: message.rootId ?? message.id,
        ),
      ).select(
        (replies) => replies.any((reply) {
          final thread = reply.threadReference;
          return thread.parentId == message.id || thread.rootId == message.id;
        }),
      ),
    );
    // Watch only this user's profile to avoid rebuilding on unrelated cache changes.
    final pk = message.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = watchChannelIdentityLabel(ref, currentChannelId, pk);
    final isAgent =
        ref.watch(agentMentionPubkeysProvider(currentChannelId)).contains(pk) ||
        profile?.ownerPubkey != null;
    final canManageMessage =
        currentPubkey?.toLowerCase() == pk ||
        (profile?.ownerPubkey != null &&
            profile?.ownerPubkey == currentPubkey?.toLowerCase());

    // Watch only profiles referenced by this message. A batched profile fetch
    // should not rebuild every visible message just because an unrelated user
    // was added to the shared cache.
    final normalizedMentionPubkeys = {
      for (final pubkey in message.mentionPubkeys) pubkey.toLowerCase(),
    };
    final mentionProfiles = <String, UserProfile?>{
      for (final pubkey in normalizedMentionPubkeys)
        pubkey: ref.watch(userCacheProvider.select((cache) => cache[pubkey])),
    };
    final knownAgentPubkeys = agentPubkeysWithProfileOwners(
      knownAgentPubkeys: ref.watch(
        agentMentionPubkeysProvider(currentChannelId),
      ),
      profileOwnedAgentPubkeys: [
        for (final entry in mentionProfiles.entries)
          if (entry.value?.ownerPubkey != null) entry.key,
      ],
    );
    final mentionNames = <String, String>{};
    final agentMentionPubkeys = <String>{};
    for (final mpk in message.mentionPubkeys) {
      final normalizedPubkey = mpk.toLowerCase();
      final p = mentionProfiles[normalizedPubkey];
      if (p?.displayName != null) {
        mentionNames[normalizedPubkey] = p!.displayName!;
      }
      if (knownAgentPubkeys.contains(normalizedPubkey)) {
        agentMentionPubkeys.add(normalizedPubkey);
      }
    }
    final resolvedMentionNames = mentionNamesWithDirectoryLabels(
      mentionPubkeys: message.mentionPubkeys,
      profileMentionNames: mentionNames,
      directoryDisplayNames: ref.watch(agentDirectoryDisplayNamesProvider),
      agentMentionPubkeys: agentMentionPubkeys,
    );
    final mentionLabels = watchChannelIdentityLabels(
      ref,
      currentChannelId,
      normalizedMentionPubkeys,
    );

    void openMessageActions(MessageLongPressDetails details) {
      showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: currentChannelId,
        canManageMessage: canManageMessage,
        allMessages: allMessages,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
        anchorRect: details.anchorRect,
        captureAnchorSnapshot: details.captureSnapshot,
        onPopoverPreviewVisibilityChanged: details.setSourceHidden,
        onPopoverDismissed: () => details.setSourceHidden(false),
        composerFocusNode: composerFocusNode,
        restoreComposerFocus: restoreComposerFocus,
      );
    }

    return NotificationListener<MessageGalleryScrollNotification>(
      onNotification: (notification) {
        mediaOffset.value = notification.offset;
        return true;
      },
      child: Padding(
        padding: EdgeInsets.only(top: showAuthor ? Grid.xs : 0),
        child: Material(
          color: Colors.transparent,
          borderRadius: BorderRadius.circular(Radii.md),
          // The media carousel intentionally continues through the list's trailing
          // gutter. InkWell still clips its ink to [borderRadius], while leaving
          // overflowing message content visible.
          clipBehavior: Clip.none,
          child: MessageLongPressInkWell(
            key: ValueKey('message-row-${message.id}'),
            onLongPressDetails: openMessageActions,
            borderRadius: BorderRadius.circular(Radii.md),
            highlightColor: context.colors.primary.withValues(alpha: 0.1),
            snapshotKey: messageSnapshotKey,
            // Tap opens existing threads; long-press can start a new one.
            // MessageContent handles mention, channel-link, and media taps.
            onTap: (!hasReplies && !hasLocalReplies) || allMessages == null
                ? null
                : () => Navigator.of(context).push(
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
                  ),
            child: Padding(
              padding: EdgeInsets.only(
                bottom: showAvatar ? Grid.quarter : Grid.quarter / 2,
              ),
              child: MessageFooterLayout(
                animateSize: true,
                overlap: message.reactions.isNotEmpty ? Grid.xs : 0,
                footer: message.reactions.isNotEmpty || replyFooter != null
                    ? Padding(
                        padding: const EdgeInsets.only(
                          left:
                              messageAvatarSize +
                              messageAvatarContentGap +
                              Grid.xxs,
                        ),
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            if (message.reactions.isNotEmpty)
                              ReactionRow(
                                combined: true,
                                messageId: message.id,
                                channelId: currentChannelId,
                                reactions: message.reactions,
                                onToggle: (emoji) =>
                                    toggleReaction(ref, message, emoji),
                                showAddButton: isMember && !isArchived,
                                onAddReaction: () => showAddReactionPicker(
                                  context: context,
                                  ref: ref,
                                  message: message,
                                ),
                              ),
                            if (replyFooter != null)
                              Padding(
                                padding: const EdgeInsets.only(
                                  bottom: Grid.xxs,
                                ),
                                child: replyFooter,
                              ),
                          ],
                        ),
                      )
                    : const SizedBox.shrink(),
                child: RepaintBoundary(
                  key: messageSnapshotKey,
                  child: Stack(
                    clipBehavior: Clip.none,
                    children: [
                      Row(
                        crossAxisAlignment: CrossAxisAlignment.end,
                        children: [
                          const SizedBox(width: messageAvatarSize),
                          const SizedBox(width: messageAvatarContentGap),
                          Expanded(
                            child: Padding(
                              padding: EdgeInsets.only(
                                top: showAuthor ? Grid.half : 0,
                              ),
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  if (showAuthor)
                                    Padding(
                                      padding: const EdgeInsets.only(
                                        bottom: Grid.quarter,
                                        left: Grid.xs,
                                      ),
                                      child: Row(
                                        children: [
                                          Expanded(
                                            child: MessageAuthorMeta(
                                              showTimestamp: true,
                                              nameStyle:
                                                  messageUsernameTextStyle
                                                      .copyWith(
                                                        fontSize: 13.1,
                                                        height: 17 / 13.1,
                                                      ),
                                              metadataStyle:
                                                  messageMetadataTextStyle
                                                      .copyWith(
                                                        fontSize: 13.1,
                                                        height: 17 / 13.1,
                                                      ),
                                              timestampStyle:
                                                  messageTimestampTextStyle
                                                      .copyWith(
                                                        fontSize: 11,
                                                        height: 17 / 11,
                                                      ),
                                              displayName: displayName,
                                              username: messageUsernameLabel(
                                                profile,
                                              ),
                                              timestamp: formatMessageTime(
                                                message.createdAt,
                                              ),
                                              nameColor:
                                                  context.colors.onSurface,
                                              metadataColor: context
                                                  .colors
                                                  .onSurfaceVariant,
                                              onAuthorTap: () =>
                                                  showUserProfileSheet(
                                                    context,
                                                    message.pubkey,
                                                    names:
                                                        channelIdentityNamesProvider(
                                                          currentChannelId,
                                                        ),
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
                                              style: context
                                                  .textTheme
                                                  .labelSmall
                                                  ?.copyWith(
                                                    color: context
                                                        .colors
                                                        .onSurfaceVariant,
                                                    fontStyle: FontStyle.italic,
                                                  ),
                                            ),
                                          ],
                                        ],
                                      ),
                                    ),
                                  AgentReplyHandoff(
                                    key: ValueKey(
                                      'reply-handoff-${message.id}',
                                    ),
                                    animate: typingHandoff,
                                    child: MessageContent(
                                      content: message.content,
                                      bubble: true,
                                      hasReactions:
                                          message.reactions.isNotEmpty ||
                                          replyFooter != null,
                                      bubbleEnd: showAvatar,
                                      bubbleStart: showAuthor,
                                      ownMessage:
                                          currentPubkey?.toLowerCase() == pk,
                                      mentionNames: resolvedMentionNames,
                                      mentionLabels: mentionLabels,
                                      agentMentionPubkeys: agentMentionPubkeys,
                                      channelNames: channelNames,
                                      tags: message.tags,
                                      baseStyle: messageBodyTextStyle.copyWith(
                                        color: context.colors.onSurface,
                                      ),
                                      scaleEmojiOnly: true,
                                      mediaCarouselTrailingOverflow:
                                          Grid.gutter,
                                      onMediaReply: allMessages == null
                                          ? null
                                          : () {
                                              if (!context.mounted) return;
                                              Navigator.of(context).push(
                                                MaterialPageRoute<void>(
                                                  builder: (_) =>
                                                      ThreadDetailPage(
                                                        threadHead: message,
                                                        allMessages:
                                                            allMessages!,
                                                        channelId:
                                                            currentChannelId,
                                                        currentPubkey:
                                                            currentPubkey,
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
                                                Navigator.of(
                                                  viewerContext,
                                                ).maybePop();
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
                                          showUserProfileSheet(
                                            context,
                                            pubkey,
                                            names: channelIdentityNamesProvider(
                                              currentChannelId,
                                            ),
                                          ),
                                    ),
                                  ),
                                ],
                              ),
                            ),
                          ),
                        ],
                      ),
                      if (showAvatar)
                        Positioned(
                          left: 0,
                          bottom: 0,
                          child: MessageMediaAvatar(
                            offset: mediaOffset,
                            child: Semantics(
                              container: true,
                              label: 'Open $displayName profile',
                              button: true,
                              excludeSemantics: true,
                              onTap: () => showUserProfileSheet(
                                context,
                                message.pubkey,
                                names: channelIdentityNamesProvider(
                                  currentChannelId,
                                ),
                              ),
                              child: GestureDetector(
                                onTap: () => showUserProfileSheet(
                                  context,
                                  message.pubkey,
                                  names: channelIdentityNamesProvider(
                                    currentChannelId,
                                  ),
                                ),
                                child: _UserAvatar(
                                  profile: profile,
                                  pubkey: message.pubkey,
                                  isAgent: isAgent,
                                ),
                              ),
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _UserAvatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;
  final bool isAgent;

  const _UserAvatar({
    required this.profile,
    required this.pubkey,
    required this.isAgent,
  });

  @override
  Widget build(BuildContext context) {
    const size = messageAvatarSize;
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return AvatarImage(
      imageUrl: avatarUrl,
      radius: size / 2,
      backgroundColor: context.controlSurface,
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
      isAgent: isAgent,
    );
  }
}

IconData channelIcon(Channel channel) {
  if (channel.isDm) return BuzzIcons.messagesSquare;
  if (channel.isPrivate) return BuzzIcons.lock;
  if (channel.isForum) return BuzzIcons.messageSquareText;
  return BuzzIcons.hash;
}
