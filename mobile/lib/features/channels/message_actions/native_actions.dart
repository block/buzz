part of '../message_actions.dart';

Future<bool> _showNativeMessageActions({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
  required bool canManageMessage,
  required List<TimelineMessage>? allMessages,
  required String? currentPubkey,
  required bool isMember,
  required bool isArchived,
  required Rect anchorRect,
  required Future<ui.Image> Function()? captureAnchorSnapshot,
  required FocusNode? composerFocusNode,
  required VoidCallback? restoreComposerFocus,
  required ValueChanged<bool>? onPopoverPreviewVisibilityChanged,
  required VoidCallback? onPopoverDismissed,
}) async {
  if (kIsWeb || defaultTargetPlatform != TargetPlatform.iOS) return false;
  return NativeMessagePresentation.withLease(() async {
    if (captureAnchorSnapshot == null) return false;
    final hadComposerFocus = composerFocusNode?.hasFocus ?? false;
    final support = await NativeMessagePresentation.present(
      'supportsMessage',
      {},
    );
    if (support?['supported'] != true) return false;
    if (!context.mounted) return true;
    final community = ref.read(relayConfigProvider);
    Uint8List? previewBytes;
    try {
      final snapshot = await captureAnchorSnapshot();
      try {
        final bytes = await snapshot.toByteData(format: ui.ImageByteFormat.png);
        previewBytes = bytes?.buffer.asUint8List();
      } finally {
        snapshot.dispose();
      }
    } catch (_) {
      return false;
    }
    if (!context.mounted || ref.read(relayConfigProvider) != community) {
      return true;
    }
    if (previewBytes == null) return false;
    final callbacks = <String, VoidCallback>{};
    final actions = <Map<String, Object?>>[];
    void action(
      String id,
      String title,
      String symbol,
      VoidCallback callback, {
      bool destructive = false,
    }) {
      callbacks[id] = callback;
      actions.add({
        'id': id,
        'title': title,
        'symbol': symbol,
        'destructive': destructive,
      });
    }

    if (!message.isSystem) {
      if (allMessages != null) {
        action('reply', 'Reply', 'arrowshape.turn.up.left', () {
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
        });
      }
      action('copyLink', 'Copy link', 'link', () {
        copyToClipboard(
          context,
          messageLinkFor(message: message, channelId: channelId),
          message: 'Message link copied',
        );
      });
      if (ref.read(reminderServiceProvider) != null) {
        action('remind', 'Remind me', 'clock', () {
          showRemindMeLaterSheet(
            context: context,
            ref: ref,
            target: ReminderTarget(
              eventId: message.id,
              channelId: channelId,
              preview: message.content.characters
                  .take(_reminderPreviewLength)
                  .toString(),
              authorPubkey: message.pubkey,
            ),
          );
        });
      }
      final readState = ref.read(readStateProvider);
      if (readState.isReady) {
        final unread = isMessageUnread(
          readState,
          channelId: channelId,
          messageId: message.id,
          createdAt: message.createdAt,
          threadRootId: message.rootId,
        );
        action('read', unread ? 'Mark read' : 'Mark unread', 'envelope', () {
          final notifier = ref.read(readStateProvider.notifier);
          if (unread) {
            notifier.markContextRead(
              msgContextKey(message.id),
              message.createdAt,
            );
          } else {
            notifier.markContextUnread(
              msgContextKey(message.id),
              channelId: channelId,
            );
          }
        });
      }
      final rootId = message.rootId ?? message.id;
      final following = ref.read(threadFollowsProvider).isFollowing(rootId);
      action(
        'follow',
        following ? 'Unfollow thread' : 'Follow thread',
        following ? 'bell.slash' : 'bell',
        () {
          final notifier = ref.read(threadFollowsProvider.notifier);
          if (following) {
            notifier.unfollowThread(rootId);
          } else {
            notifier.followThread(rootId);
          }
        },
      );
      action('copy', 'Copy text', 'doc.on.doc', () {
        Clipboard.setData(ClipboardData(text: message.content));
      });
    }
    if (canManageMessage) {
      action('edit', 'Edit message', 'pencil', () {
        _showEditSheet(
          context: context,
          ref: ref,
          message: message,
          channelId: channelId,
        );
      });
      action('delete', 'Delete message', 'trash', () {
        _confirmDelete(
          context: context,
          ref: ref,
          channelId: channelId,
          messageId: message.id,
        );
      }, destructive: true);
    }
    final palette = ref.read(customEmojiListProvider);
    final emojis = quickReactionEmoji(
      ref.read(recentEmojiProvider),
      customShortcodes: {for (final e in palette) e.shortcode.toLowerCase()},
    );
    final dataset = ref.read(emojiDatasetOrEmptyProvider);
    Map<Object?, Object?>? response;
    var ownsPreview = false;
    if (hadComposerFocus) composerFocusNode!.unfocus();
    try {
      response = await NativeMessagePresentation.present(
        'message',
        {
          'x': anchorRect.left,
          'y': anchorRect.top,
          'width': anchorRect.width,
          'height': anchorRect.height,
          'dark': Theme.of(context).brightness == Brightness.dark,
          'previewLabel': message.content,
          'previewBytes': previewBytes,
          'actions': actions,
          'reactions': [
            for (final emoji in emojis)
              {
                'emoji': emoji,
                'label': dataset.displayName(emoji),
                'selected': message.reactions.any(
                  (r) => r.emoji == emoji && r.reactedByCurrentUser,
                ),
                if (reactionEmojiUrl(emoji, palette) case final String url) ...{
                  'url': url,
                  'headers': mediaGetHeadersFor(ref, url),
                },
              },
          ],
        },
        onPresented: () {
          if (!context.mounted || ownsPreview) return;
          ownsPreview = true;
          onPopoverPreviewVisibilityChanged?.call(true);
        },
      );
    } finally {
      if (ownsPreview) {
        onPopoverPreviewVisibilityChanged?.call(false);
        if (context.mounted) onPopoverDismissed?.call();
      }
      if (hadComposerFocus &&
          response?['busy'] != true &&
          response?['action'] == null &&
          context.mounted) {
        restoreComposerFocus?.call();
      }
    }
    if (response == null) return false;
    // A native controller can outlive the Flutter page that opened it.
    if (!context.mounted || ref.read(relayConfigProvider) != community) {
      return true;
    }
    final selected = response['action'];
    if (selected == 'reaction' && emojis.contains(response['emoji'])) {
      final emoji = response['emoji']! as String;
      final existing = message.reactions.where((r) => r.emoji == emoji);
      if (existing.isNotEmpty && existing.first.reactedByCurrentUser) {
        toggleReaction(ref, message, emoji);
      } else {
        ref.read(recentEmojiProvider.notifier).record(emoji);
        armReactionBurst(ref, message, emoji);
        ref.read(channelActionsProvider).addReaction(message.id, emoji);
      }
    } else if (selected == 'more') {
      showAddReactionPicker(context: context, ref: ref, message: message);
    } else {
      callbacks[selected]?.call();
    }
    return true;
  });
}
