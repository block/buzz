import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import 'channel_management_provider.dart';
import 'emoji_picker.dart';
import 'thread_detail_page.dart';
import 'timeline_message.dart';

const quickEmojis = [
  '\u{1F44D}',
  '\u{2764}\u{FE0F}',
  '\u{1F602}',
  '\u{1F389}',
  '\u{1F440}',
  '\u{1F64F}',
];

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
    showDragHandle: true,
    builder: (sheetContext) => SafeArea(
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
                  GestureDetector(
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      ref
                          .read(channelActionsProvider)
                          .addReaction(message.id, emoji);
                    },
                    child: Container(
                      width: 44,
                      height: 44,
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: sheetContext.colors.surfaceContainerHighest,
                        shape: BoxShape.circle,
                      ),
                      child: Text(emoji, style: const TextStyle(fontSize: 20)),
                    ),
                  ),
                GestureDetector(
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
                  child: Container(
                    width: 44,
                    height: 44,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: sheetContext.colors.surfaceContainerHighest,
                      shape: BoxShape.circle,
                    ),
                    child: Icon(
                      LucideIcons.plus,
                      size: 20,
                      color: sheetContext.colors.onSurfaceVariant,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: Grid.xs),
            if (allMessages != null && !message.isSystem)
              ListTile(
                leading: const Icon(LucideIcons.messageSquareReply),
                title: const Text('Reply in thread'),
                onTap: () {
                  Navigator.of(sheetContext).pop();
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
              ),
            if (!message.isSystem)
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
            if (canManageMessage) ...[
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
  );
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
