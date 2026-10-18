part of '../thread_detail_page.dart';

class _ThreadMessage extends HookConsumerWidget {
  final TimelineMessage message;
  final Map<String, String> channelNames;
  final String channelId;
  final String? currentPubkey;
  final bool showAuthor;
  final bool showAvatar;
  final bool startsDay;
  final bool endsDay;
  final bool isHighlighted;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;
  final FocusNode? composerFocusNode;
  final VoidCallback? restoreComposerFocus;

  /// Whether this is the message the thread hangs off, which keeps a standing
  /// "+" where replies only get one once they carry a reaction.
  final bool isThreadHead;

  const _ThreadMessage({
    required this.message,
    required this.channelNames,
    required this.channelId,
    required this.currentPubkey,
    required this.showAuthor,
    this.showAvatar = true,
    this.startsDay = false,
    this.endsDay = false,
    this.isHighlighted = false,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
    this.isThreadHead = false,
    this.composerFocusNode,
    this.restoreComposerFocus,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final messageSnapshotKey = useMemoized(GlobalKey.new, const []);
    final swipeContentKey = useMemoized(GlobalKey.new, const []);
    final mediaOffset = useMessageMediaOffset(message.content, message.tags);
    final pk = message.pubkey.toLowerCase();
    final ownMessage = currentPubkey?.toLowerCase() == pk;
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = watchChannelIdentityLabel(ref, channelId, pk);
    final isAgent =
        ref.watch(agentMentionPubkeysProvider(channelId)).contains(pk) ||
        profile?.ownerPubkey != null;
    final canManageMessage =
        currentPubkey?.toLowerCase() == pk ||
        (profile?.ownerPubkey != null &&
            profile?.ownerPubkey == currentPubkey?.toLowerCase());

    final userCache = ref.watch(userCacheProvider);
    final knownAgentPubkeys = agentPubkeysWithProfileOwners(
      knownAgentPubkeys: ref.watch(agentMentionPubkeysProvider(channelId)),
      profileOwnedAgentPubkeys: [
        for (final profile in userCache.values)
          if (profile.ownerPubkey != null) profile.pubkey,
      ],
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
    final resolvedMentionNames = mentionNamesWithDirectoryLabels(
      mentionPubkeys: message.mentionPubkeys,
      profileMentionNames: mentionNames,
      directoryDisplayNames: ref.watch(agentDirectoryDisplayNamesProvider),
      agentMentionPubkeys: agentMentionPubkeys,
    );
    final mentionLabels = watchChannelIdentityLabels(
      ref,
      channelId,
      message.mentionPubkeys,
    );

    void openMessageActions(MessageLongPressDetails details) {
      showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: channelId,
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

    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final highlightController = useAnimationController(
      duration: _landingHighlightTransitionDuration,
    );
    final highlightProgress = useAnimation(highlightController);
    useEffect(() {
      if (reducedMotion) {
        highlightController.value = isHighlighted ? 1 : 0;
      } else {
        unawaited(
          highlightController.animateTo(
            isHighlighted ? 1 : 0,
            duration: _landingHighlightTransitionDuration,
            curve: Curves.easeOutCubic,
          ),
        );
      }
      return null;
    }, [highlightController, isHighlighted, reducedMotion]);
    final highlightColor = highlightProgress == 0
        ? Colors.transparent
        : context.colors.primary.withValues(
            alpha: _landingHighlightOpacity * highlightProgress,
          );

    final content = NotificationListener<MessageGalleryScrollNotification>(
      onNotification: (notification) {
        mediaOffset.value = notification.offset;
        return true;
      },
      child: Padding(
        padding: EdgeInsets.only(top: showAuthor && !startsDay ? Grid.xs : 0),
        child: DecoratedBox(
          key: ValueKey('thread-message-${message.id}'),
          decoration: BoxDecoration(
            color: highlightColor,
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          child: Material(
            color: Colors.transparent,
            borderRadius: BorderRadius.circular(Radii.md),
            // The media carousel intentionally continues through the list's
            // trailing gutter. InkWell still clips its ink to [borderRadius],
            // while leaving overflowing message content visible.
            clipBehavior: Clip.none,
            child: MessageLongPressInkWell(
              key: ValueKey('thread-message-row-${message.id}'),
              onLongPressDetails: openMessageActions,
              borderRadius: BorderRadius.circular(Radii.md),
              highlightColor: context.colors.primary.withValues(alpha: 0.1),
              snapshotKey: messageSnapshotKey,
              child: Padding(
                padding: EdgeInsets.only(
                  bottom: endsDay
                      ? 0
                      : (showAvatar ? Grid.quarter : Grid.quarter / 2),
                ),
                child: MessageFooterLayout(
                  animateSize: true,
                  overlap: message.reactions.isNotEmpty ? Grid.xs : 0,
                  footer: (isThreadHead || message.reactions.isNotEmpty)
                      ? Align(
                          alignment: ownMessage
                              ? Alignment.centerRight
                              : Alignment.centerLeft,
                          child: Padding(
                            padding: EdgeInsets.only(
                              right: ownMessage ? Grid.xxs : 0,
                              left: ownMessage
                                  ? 0
                                  : messageAvatarSize +
                                        messageAvatarContentGap +
                                        Grid.xxs,
                            ),
                            child: ReactionRow(
                              combined: true,
                              messageId: message.id,
                              channelId: channelId,
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
                          ),
                        )
                      : const SizedBox.shrink(),
                  child: RepaintBoundary(
                    key: messageSnapshotKey,
                    child: Stack(
                      clipBehavior: Clip.none,
                      children: [
                        Row(
                          textDirection: ownMessage
                              ? TextDirection.rtl
                              : TextDirection.ltr,
                          crossAxisAlignment: CrossAxisAlignment.end,
                          children: [
                            if (!ownMessage)
                              const SizedBox(width: messageAvatarSize),
                            if (!ownMessage)
                              const SizedBox(width: messageAvatarContentGap),
                            Expanded(
                              child: Padding(
                                padding: EdgeInsets.only(
                                  top: showAuthor && !startsDay ? Grid.half : 0,
                                ),
                                child: Column(
                                  crossAxisAlignment: ownMessage
                                      ? CrossAxisAlignment.end
                                      : CrossAxisAlignment.start,
                                  children: [
                                    if (showAuthor)
                                      Padding(
                                        padding: EdgeInsets.only(
                                          bottom: Grid.quarter,
                                          left: ownMessage ? 0 : Grid.xs,
                                          right: ownMessage ? Grid.xs : 0,
                                        ),
                                        child: Row(
                                          children: [
                                            Expanded(
                                              child: MessageAuthorMeta(
                                                showTimestamp: true,
                                                reversed: false,
                                                alignment: ownMessage
                                                    ? MainAxisAlignment.end
                                                    : MainAxisAlignment.start,
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
                                                            channelId,
                                                          ),
                                                    ),
                                                displayNameKey: ValueKey(
                                                  'thread-message-author-${message.id}',
                                                ),
                                                usernameKey: ValueKey(
                                                  'thread-message-username-${message.id}',
                                                ),
                                                timestampKey: ValueKey(
                                                  'thread-message-timestamp-${message.id}',
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
                                                      fontStyle:
                                                          FontStyle.italic,
                                                    ),
                                              ),
                                            ],
                                          ],
                                        ),
                                      ),
                                    MessageContent(
                                      key: swipeContentKey,
                                      content: message.content,
                                      bubble: true,
                                      hasReactions:
                                          message.reactions.isNotEmpty,
                                      bubbleEnd: showAvatar,
                                      bubbleStart: showAuthor,
                                      ownMessage: ownMessage,
                                      trimTrailingSpacing: endsDay,
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
                                                        channelId: channelId,
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
                                            channelId: channelId,
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
                                      onChannelTap: (targetChannelId) {
                                        openChannelLink(
                                          context: context,
                                          ref: ref,
                                          channelId: targetChannelId,
                                          currentChannelId: channelId,
                                        );
                                      },
                                      onMentionTap: (pubkey) =>
                                          showUserProfileSheet(
                                            context,
                                            pubkey,
                                            names: channelIdentityNamesProvider(
                                              channelId,
                                            ),
                                          ),
                                    ),
                                  ],
                                ),
                              ),
                            ),
                          ],
                        ),
                        if (showAvatar && !ownMessage)
                          Positioned(
                            left: ownMessage ? null : 0,
                            right: ownMessage ? 0 : null,
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
                                    channelId,
                                  ),
                                ),
                                child: GestureDetector(
                                  onTap: () => showUserProfileSheet(
                                    context,
                                    message.pubkey,
                                    names: channelIdentityNamesProvider(
                                      channelId,
                                    ),
                                  ),
                                  child: _Avatar(
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
      ),
    );
    return MessageSwipeReply(
      contentKey: swipeContentKey,
      message: message,
      allMessages: allMessages,
      channelId: channelId,
      currentPubkey: currentPubkey,
      isMember: isMember,
      isArchived: isArchived,
      child: content,
    );
  }
}
