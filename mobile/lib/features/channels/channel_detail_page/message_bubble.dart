part of '../channel_detail_page.dart';

class _MessageBubble extends HookConsumerWidget {
  final TimelineMessage message;
  final bool showAuthor;
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
    // Watch only this user's profile to avoid rebuilding on unrelated cache changes.
    final pk = message.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? shortPubkey(message.pubkey);
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

    return Padding(
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
          // Tap opens the thread; long-press still opens the action sheet.
          // MessageContent handles mention, channel-link, and media taps.
          onTap: allMessages == null
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
              top: showAuthor ? 0 : Grid.xxs,
              bottom: showAuthor ? 0 : Grid.xxs,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                RepaintBoundary(
                  key: messageSnapshotKey,
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (showAuthor)
                        GestureDetector(
                          onTap: () =>
                              showUserProfileSheet(context, message.pubkey),
                          child: _UserAvatar(
                            profile: profile,
                            pubkey: message.pubkey,
                            isAgent: isAgent,
                          ),
                        )
                      else
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
                                  ),
                                  child: Row(
                                    children: [
                                      Expanded(
                                        child: MessageAuthorMeta(
                                          displayName: displayName,
                                          username: messageUsernameLabel(
                                            profile,
                                          ),
                                          timestamp: formatMessageTime(
                                            message.createdAt,
                                          ),
                                          nameColor: context.colors.onSurface,
                                          metadataColor:
                                              context.colors.onSurfaceVariant,
                                          onAuthorTap: () =>
                                              showUserProfileSheet(
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
                                          style: context.textTheme.labelSmall
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
                              MessageContent(
                                content: message.content,
                                mentionNames: resolvedMentionNames,
                                agentMentionPubkeys: agentMentionPubkeys,
                                channelNames: channelNames,
                                tags: message.tags,
                                baseStyle: messageBodyTextStyle.copyWith(
                                  color: context.colors.onSurface,
                                ),
                                scaleEmojiOnly: true,
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
                                    showUserProfileSheet(context, pubkey),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                if (message.reactions.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(
                      left: messageAvatarSize + messageAvatarContentGap,
                    ),
                    child: ReactionRow(
                      messageId: message.id,
                      reactions: message.reactions,
                      onToggle: (emoji) => toggleReaction(ref, message, emoji),
                      showAddButton: isMember && !isArchived,
                      onAddReaction: () => showAddReactionPicker(
                        context: context,
                        ref: ref,
                        message: message,
                      ),
                    ),
                  ),
              ],
            ),
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
  final bool isAgent;
  final double size;

  const _UserAvatar({
    required this.profile,
    required this.pubkey,
    required this.isAgent,
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
      isAgent: isAgent,
    );
  }
}

IconData channelIcon(Channel channel) {
  if (channel.isDm) return LucideIcons.messagesSquare;
  if (channel.isPrivate) return LucideIcons.lock;
  if (channel.isForum) return LucideIcons.messageSquareText;
  return LucideIcons.hash;
}
