import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/deeplink/deep_link.dart';
import '../../shared/theme/theme.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/widgets/sheet_divider.dart';
import '../../shared/reminders/remind_me_later_sheet.dart';
import '../../shared/reminders/reminder_service.dart';
import 'channel_management_provider.dart';
import 'emoji_picker.dart';
import 'read_state/message_read_state.dart';
import 'read_state/read_state_format.dart';
import 'read_state/read_state_provider.dart';
import 'thread_detail_page.dart';
import 'thread_follows/thread_follows_provider.dart';
import 'timeline_message.dart';

const quickEmojis = ['\u{1F44D}', '\u{2764}\u{FE0F}', '\u{1F602}', '\u{1F389}'];

/// Preview length for reminder targets — matches desktop's
/// `msg.body.slice(0, 100)`.
const _reminderPreviewLength = 100;

void showMessageActions({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
  required bool canManageMessage,
  List<TimelineMessage>? allMessages,
  String? currentPubkey,
  bool isMember = false,
  bool isArchived = false,
}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (sheetContext) => SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(sheetContext).height * 0.7,
        ),
        child: SingleChildScrollView(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                // Quick emoji row
                Row(
                  mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                  children: [
                    for (final emoji in quickEmojis)
                      _QuickReactionCircle(
                        onTap: () {
                          Navigator.of(sheetContext).pop();
                          ref
                              .read(channelActionsProvider)
                              .addReaction(message.id, emoji);
                        },
                        child: Text(
                          emoji,
                          style: const TextStyle(fontSize: 24),
                        ),
                      ),
                    _QuickReactionCircle(
                      onTap: () {
                        Navigator.of(sheetContext).pop();
                        showEmojiPicker(
                          context: context,
                          onSelect: (emoji) {
                            ref
                                .read(channelActionsProvider)
                                .addReaction(message.id, emoji);
                          },
                        );
                      },
                      child: Icon(
                        LucideIcons.plus,
                        size: 24,
                        color: sheetContext.colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: Grid.xs),
                if (!message.isSystem) ...[
                  // Fast actions: respond now, hand off context, defer.
                  _FastActionsRow(
                    message: message,
                    channelId: channelId,
                    allMessages: allMessages,
                    currentPubkey: currentPubkey,
                    isMember: isMember,
                    isArchived: isArchived,
                    pageContext: context,
                  ),
                  const SizedBox(height: Grid.xs),
                  // Triage: come back to this message later.
                  _MarkReadUnreadTile(message: message, channelId: channelId),
                  _FollowThreadTile(message: message),
                  const SheetDivider(),
                  // Export: take the content out of the conversation.
                  ListTile(
                    leading: const Icon(LucideIcons.copy),
                    title: const Text('Copy text'),
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      // Copy to clipboard
                      final data = ClipboardData(text: message.content);
                      Clipboard.setData(data);
                    },
                  ),
                ],
                if (canManageMessage) ...[
                  if (!message.isSystem) const SheetDivider(),
                  ListTile(
                    leading: const Icon(LucideIcons.pencil),
                    title: const Text('Edit message'),
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      _showEditSheet(
                        context: context,
                        ref: ref,
                        message: message,
                        channelId: channelId,
                      );
                    },
                  ),
                  ListTile(
                    leading: Icon(
                      LucideIcons.trash2,
                      color: sheetContext.colors.error,
                    ),
                    title: Text(
                      'Delete message',
                      style: TextStyle(color: sheetContext.colors.error),
                    ),
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      _confirmDelete(
                        context: context,
                        ref: ref,
                        channelId: channelId,
                        messageId: message.id,
                      );
                    },
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

/// Canonical `buzz://message` link for a timeline message, including thread
/// context when the message is a reply.
String messageLinkFor({
  required TimelineMessage message,
  required String channelId,
}) {
  return buildMessageLink(
    channelId: channelId,
    messageId: message.id,
    threadRootId: message.rootId,
  );
}

class _MarkReadUnreadTile extends ConsumerWidget {
  final TimelineMessage message;
  final String channelId;

  const _MarkReadUnreadTile({required this.message, required this.channelId});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final readState = ref.watch(readStateProvider);
    if (!readState.isReady) return const SizedBox.shrink();

    final unread = isMessageUnread(
      readState,
      channelId: channelId,
      messageId: message.id,
      createdAt: message.createdAt,
      threadRootId: message.rootId,
    );

    return ListTile(
      leading: Icon(unread ? LucideIcons.mailCheck : LucideIcons.mailOpen),
      title: Text(unread ? 'Mark read' : 'Mark unread'),
      onTap: () {
        Navigator.of(context).pop();
        final notifier = ref.read(readStateProvider.notifier);
        if (unread) {
          // Advances the `msg:` marker and clears this message's own forced
          // flag — a channel-level forced unread (channel tile) is a separate
          // choice and stays untouched.
          notifier.markContextRead(
            msgContextKey(message.id),
            message.createdAt,
          );
        } else {
          // Read markers are monotonic and cannot move backward, so mark
          // unread forces this message unread locally (session-local, does
          // not survive relaunch). The channel surfaces it as unread too.
          notifier.markContextUnread(
            msgContextKey(message.id),
            channelId: channelId,
          );
        }
      },
    );
  }
}

class _FollowThreadTile extends ConsumerWidget {
  final TimelineMessage message;

  const _FollowThreadTile({required this.message});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final follows = ref.watch(threadFollowsProvider);
    // Follow acts on the effective thread root: the root for replies, the
    // message itself for potential thread heads (matches desktop).
    final rootId = message.rootId ?? message.id;
    final following = follows.isFollowing(rootId);

    return ListTile(
      leading: Icon(following ? LucideIcons.bellOff : LucideIcons.bellRing),
      title: Text(following ? 'Unfollow thread' : 'Follow thread'),
      onTap: () {
        Navigator.of(context).pop();
        final notifier = ref.read(threadFollowsProvider.notifier);
        if (following) {
          notifier.unfollowThread(rootId);
        } else {
          notifier.followThread(rootId);
        }
      },
    );
  }
}

/// Promoted actions for the three dominant mobile jobs: respond now (Reply),
/// hand off context (Copy link — the `buzz://message` link is the workspace's
/// context-transfer primitive), and defer (Remind me).
class _FastActionsRow extends ConsumerWidget {
  final TimelineMessage message;
  final String channelId;
  final List<TimelineMessage>? allMessages;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  /// The long-pressed message's page context — survives the sheet pop, used
  /// for the thread push and the copy-link snackbar.
  final BuildContext pageContext;

  const _FastActionsRow({
    required this.message,
    required this.channelId,
    required this.allMessages,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
    required this.pageContext,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final messages = allMessages;
    final canRemind = ref.watch(reminderServiceProvider) != null;

    final tiles = <Widget>[
      if (messages != null)
        _FastActionTile(
          icon: LucideIcons.messageSquareReply,
          label: 'Reply',
          onTap: () {
            Navigator.of(context).pop();
            Navigator.of(pageContext).push(
              MaterialPageRoute<void>(
                builder: (_) => ThreadDetailPage(
                  threadHead: message,
                  allMessages: messages,
                  channelId: channelId,
                  currentPubkey: currentPubkey,
                  isMember: isMember,
                  isArchived: isArchived,
                ),
              ),
            );
          },
        ),
      _FastActionTile(
        icon: LucideIcons.link2,
        label: 'Copy link',
        onTap: () {
          Navigator.of(context).pop();
          copyToClipboard(
            pageContext,
            messageLinkFor(message: message, channelId: channelId),
            message: 'Message link copied',
          );
        },
      ),
      if (canRemind)
        _FastActionTile(
          icon: LucideIcons.clock,
          label: 'Remind me',
          onTap: () {
            final rootContext = Navigator.of(
              context,
              rootNavigator: true,
            ).context;
            Navigator.of(context).pop();
            showRemindMeLaterSheet(
              context: rootContext,
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
          },
        ),
    ];

    return Row(
      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
      children: tiles,
    );
  }
}

class _FastActionTile extends StatelessWidget {
  final IconData icon;
  final String label;
  final VoidCallback onTap;

  const _FastActionTile({
    required this.icon,
    required this.label,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          // Pill-shaped icon container with the caption below the fill,
          // matching the emoji circles' fill and tap treatment.
          Container(
            width: 76,
            height: 56,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: context.colors.surfaceContainerHighest,
              borderRadius: BorderRadius.circular(Radii.dialog),
            ),
            child: Icon(icon, size: 24, color: context.colors.onSurface),
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: context.textTheme.labelMedium?.copyWith(
              color: context.colors.onSurface,
            ),
          ),
        ],
      ),
    );
  }
}

/// Circular filled tap target shared by the quick-reaction emojis and the
/// "+" emoji-picker tile — one treatment, one place to change it.
class _QuickReactionCircle extends StatelessWidget {
  final VoidCallback onTap;
  final Widget child;

  const _QuickReactionCircle({required this.onTap, required this.child});

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        width: 52,
        height: 52,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: context.colors.surfaceContainerHighest,
          shape: BoxShape.circle,
        ),
        child: child,
      ),
    );
  }
}

void _showEditSheet({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
}) {
  final controller = TextEditingController(text: message.content);
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (sheetContext) => Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        MediaQuery.viewInsetsOf(sheetContext).bottom,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          TextField(
            controller: controller,
            autofocus: true,
            minLines: 1,
            maxLines: 5,
            decoration: const InputDecoration(hintText: 'Edit message'),
          ),
          const SizedBox(height: Grid.xxs),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              TextButton(
                onPressed: () => Navigator.of(sheetContext).pop(),
                child: const Text('Cancel'),
              ),
              const SizedBox(width: Grid.half),
              FilledButton(
                onPressed: () {
                  final text = controller.text.trim();
                  if (text.isEmpty || text == message.content) {
                    Navigator.of(sheetContext).pop();
                    return;
                  }
                  ref
                      .read(channelActionsProvider)
                      .editMessage(
                        channelId: channelId,
                        eventId: message.id,
                        content: text,
                        mediaTags: buildCustomEmojiTags(
                          text,
                          ref.read(customEmojiListProvider),
                        ),
                      );
                  Navigator.of(sheetContext).pop();
                },
                child: const Text('Save'),
              ),
            ],
          ),
        ],
      ),
    ),
  );
}

void _confirmDelete({
  required BuildContext context,
  required WidgetRef ref,
  required String channelId,
  required String messageId,
}) {
  showDialog<void>(
    context: context,
    builder: (dialogContext) => AlertDialog(
      title: const Text('Delete message'),
      content: const Text('This cannot be undone.'),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () {
            Navigator.of(dialogContext).pop();
            final messenger = ScaffoldMessenger.of(context);
            ref
                .read(channelActionsProvider)
                .deleteMessage(channelId: channelId, eventId: messageId)
                .catchError((Object error) {
                  messenger.showSnackBar(
                    SnackBar(content: Text('Failed to delete message: $error')),
                  );
                });
          },
          style: FilledButton.styleFrom(
            backgroundColor: dialogContext.colors.error,
          ),
          child: const Text('Delete'),
        ),
      ],
    ),
  );
}
