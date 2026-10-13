import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:path_provider/path_provider.dart';
import 'package:photo_manager/photo_manager.dart';
import 'package:share_plus/share_plus.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/deeplink/deep_link.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/emoji/native_emoji_glyph.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/widgets/sheet_action_section.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/reminders/remind_me_later_sheet.dart';
import '../../shared/reminders/reminder_service.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';
import 'emoji_picker.dart';
import 'reaction_row.dart';
import 'recent_emoji_provider.dart';
import '../../shared/read_state/message_read_state.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'thread_detail_page.dart';
import 'thread_follows/thread_follows_provider.dart';
import 'timeline_message.dart';
import 'unread_badge/is_high_priority_event.dart';

part 'message_actions/quick_reaction_row.dart';

/// Preview length for reminder targets — matches desktop's
/// `msg.body.slice(0, 100)`.
const _reminderPreviewLength = 100;

// A weak, route-owned latch shared by every action, including pointers that
// were already down when another selection started dismissing the sheet.
final _selectedMessageActionRoutes = Expando<bool>();

bool _selectMessageAction(BuildContext context) {
  final route = ModalRoute.of(context);
  if (route == null ||
      !route.isCurrent ||
      _selectedMessageActionRoutes[route] == true) {
    return false;
  }
  _selectedMessageActionRoutes[route] = true;
  Navigator.of(context).pop(true);
  return true;
}

/// Presents message reactions and grouped actions in the shared bottom sheet.
/// Snapshot and preview callbacks are not used by the sheet presentation.
Future<void> showMessageActions({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
  required bool canManageMessage,
  List<TimelineMessage>? allMessages,
  String? currentPubkey,
  bool isMember = false,
  Rect? anchorRect,
  Future<ui.Image> Function()? captureAnchorSnapshot,
  ValueChanged<bool>? onPopoverPreviewVisibilityChanged,
  VoidCallback? onPopoverDismissed,
  FocusNode? composerFocusNode,
  VoidCallback? restoreComposerFocus,
  bool isArchived = false,
  EdgeInsets popoverSpotlightPadding = const EdgeInsets.all(Grid.xxs),
}) async {
  if (!context.mounted) return;
  unawaited(HapticFeedback.mediumImpact());
  final restoreFocusOnCancel = composerFocusNode?.hasFocus ?? false;
  composerFocusNode?.unfocus();

  final selectedAction = await showBuzzModalBottomSheet<bool>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    showCloseButton: false,
    constraints: BoxConstraints(
      maxWidth: 640,
      maxHeight: MediaQuery.sizeOf(context).height * 0.7,
    ),
    builder: (sheetContext) => SafeArea(
      top: false,
      child: IconTheme.merge(
        data: const IconThemeData(size: 22),
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
                  _QuickReactionRow(
                    message: message,
                    sheetContext: sheetContext,
                    pageContext: context,
                    pageRef: ref,
                  ),
                  SizedBox(height: message.isSystem ? Grid.xxs : Grid.xs),
                  if (!message.isSystem) ...[
                    // Fast actions: respond now, hand off context, defer.
                    _MessageQuickActionsRow(
                      message: message,
                      channelId: channelId,
                      allMessages: allMessages,
                      currentPubkey: currentPubkey,
                      isMember: isMember,
                      isArchived: isArchived,
                      pageContext: context,
                    ),
                    // Section cards supply the other half of the 16pt gap.
                    const SizedBox(height: Grid.xxs),
                    // Triage: come back to this message later.
                    SheetActionSection(
                      children: [
                        _MarkReadUnreadTile(
                          message: message,
                          channelId: channelId,
                          currentPubkey: currentPubkey,
                        ),
                        _FollowThreadTile(message: message),
                      ],
                    ),
                    SheetActionSection(
                      children: [
                        // Export: take the content out of the conversation.
                        ListTile(
                          leading: const Icon(BuzzIcons.copy),
                          title: const Text('Copy text'),
                          onTap: () {
                            if (!_selectMessageAction(sheetContext)) return;
                            // Copy to clipboard
                            final data = ClipboardData(text: message.content);
                            Clipboard.setData(data);
                          },
                        ),
                      ],
                    ),
                  ],
                  if (canManageMessage)
                    SheetActionSection(
                      children: [
                        ListTile(
                          leading: const Icon(BuzzIcons.pencil),
                          title: const Text('Edit message'),
                          onTap: () {
                            if (!_selectMessageAction(sheetContext)) return;
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
                            BuzzIcons.trash2,
                            color: sheetContext.colors.error,
                          ),
                          title: Text(
                            'Delete message',
                            style: TextStyle(color: sheetContext.colors.error),
                          ),
                          onTap: () {
                            if (!_selectMessageAction(sheetContext)) return;
                            _confirmDelete(
                              context: context,
                              ref: ref,
                              channelId: channelId,
                              messageId: message.id,
                            );
                          },
                        ),
                      ],
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    ),
  );
  if (selectedAction != true && restoreFocusOnCancel && context.mounted) {
    restoreComposerFocus?.call();
  }
}

/// Image-focused actions shown from the full-screen viewer.
void showImageActions({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
  required String imageUrl,
  required bool canManageMessage,
  VoidCallback? onDeleted,
}) {
  showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    title: 'Image',
    builder: (sheetContext) => SafeArea(
      child: SingleChildScrollView(
        child: IconTheme.merge(
          data: const IconThemeData(size: 22),
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
                SheetActionSection(
                  children: [
                    ListTile(
                      leading: const Icon(BuzzIcons.download),
                      title: const Text('Save image'),
                      onTap: () {
                        Navigator.of(sheetContext).pop();
                        unawaited(_saveImage(context, ref, imageUrl));
                      },
                    ),
                    ListTile(
                      leading: const Icon(BuzzIcons.share2),
                      title: const Text('Share image'),
                      onTap: () {
                        final renderBox =
                            context.findRenderObject() as RenderBox?;
                        final shareOrigin = renderBox == null
                            ? null
                            : renderBox.localToGlobal(Offset.zero) &
                                  renderBox.size;
                        Navigator.of(sheetContext).pop();
                        unawaited(
                          _shareImage(
                            context,
                            ref,
                            imageUrl,
                            shareOrigin: shareOrigin,
                          ),
                        );
                      },
                    ),
                    ListTile(
                      leading: const Icon(BuzzIcons.link2),
                      title: const Text('Copy image link'),
                      onTap: () {
                        Navigator.of(sheetContext).pop();
                        copyToClipboard(
                          context,
                          imageUrl,
                          message: 'Image link copied',
                        );
                      },
                    ),
                  ],
                ),
                if (canManageMessage)
                  SheetActionSection(
                    children: [
                      ListTile(
                        leading: Icon(
                          BuzzIcons.trash2,
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
                            onDeleted: onDeleted,
                          );
                        },
                      ),
                    ],
                  ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

@immutable
class _DownloadedImage {
  final Uint8List bytes;
  final String filename;

  const _DownloadedImage({required this.bytes, required this.filename});
}

Future<_DownloadedImage> _downloadImage(WidgetRef ref, String imageUrl) async {
  final response = await ref
      .read(mediaHttpClientProvider)
      .get(
        Uri.parse(imageUrl),
        headers: ref.read(mediaGetAuthServiceProvider).headersFor(imageUrl),
      );
  if (response.statusCode < 200 || response.statusCode >= 300) {
    throw HttpException(
      'Image download failed (${response.statusCode})',
      uri: Uri.parse(imageUrl),
    );
  }
  return _DownloadedImage(
    bytes: response.bodyBytes,
    filename: downloadedImageFilename(
      imageUrl,
      response.headers['content-type'],
    ),
  );
}

/// Returns a safe image filename while preserving supported image formats.
@visibleForTesting
String downloadedImageFilename(String imageUrl, String? contentType) {
  final pathSegments = Uri.tryParse(imageUrl)?.pathSegments;
  final rawName = pathSegments == null || pathSegments.isEmpty
      ? ''
      : pathSegments.last;
  final safeName = rawName
      .replaceAll(RegExp(r'[^A-Za-z0-9._-]'), '-')
      .replaceAll(RegExp(r'-+'), '-');
  if (RegExp(
    r'\.(avif|bmp|gif|heic|heif|jpe?g|png|webp)$',
    caseSensitive: false,
  ).hasMatch(safeName)) {
    return safeName;
  }
  final extension = switch (contentType?.split(';').first.trim()) {
    'image/avif' => '.avif',
    'image/gif' => '.gif',
    'image/heic' => '.heic',
    'image/heif' => '.heif',
    'image/png' => '.png',
    'image/webp' => '.webp',
    _ => '.jpg',
  };
  return 'buzz-${DateTime.now().millisecondsSinceEpoch}$extension';
}

Future<void> _saveImage(
  BuildContext context,
  WidgetRef ref,
  String imageUrl,
) async {
  final messenger = ScaffoldMessenger.maybeOf(context);
  try {
    final needsPhotoLibraryPermission =
        defaultTargetPlatform == TargetPlatform.iOS ||
        await requiresLegacyMediaStoragePermission();
    if (needsPhotoLibraryPermission) {
      final permission = await PhotoManager.requestPermissionExtend(
        requestOption: const PermissionRequestOption(
          iosAccessLevel: IosAccessLevel.addOnly,
          androidPermission: AndroidPermission(
            type: RequestType.image,
            mediaLocation: false,
          ),
        ),
      );
      if (!permission.isAuth) {
        throw const FileSystemException(
          'Photo library permission was not granted.',
        );
      }
    }
    final image = await _downloadImage(ref, imageUrl);
    await PhotoManager.editor.saveImage(image.bytes, filename: image.filename);
    messenger?.showSnackBar(
      const SnackBar(content: Text('Image saved to Photos')),
    );
  } catch (_) {
    messenger?.showSnackBar(
      const SnackBar(content: Text('Could not save image')),
    );
  }
}

Future<void> _shareImage(
  BuildContext context,
  WidgetRef ref,
  String imageUrl, {
  Rect? shareOrigin,
}) async {
  final messenger = ScaffoldMessenger.maybeOf(context);
  try {
    final image = await _downloadImage(ref, imageUrl);
    final directory = await getTemporaryDirectory();
    final file = File(
      '${directory.path}${Platform.pathSeparator}${image.filename}',
    );
    await file.writeAsBytes(image.bytes, flush: true);
    await SharePlus.instance.share(
      ShareParams(files: [XFile(file.path)], sharePositionOrigin: shareOrigin),
    );
  } catch (_) {
    messenger?.showSnackBar(
      const SnackBar(content: Text('Could not share image')),
    );
  }
}

/// Whether the actions menu should offer **Mark read** for `message`.
///
/// This is the one decision every menu presentation (sheet, popover, native)
/// uses. Mentions are classified with the signing key, not the optional
/// profile, so a user without a published profile sees the same read state
/// as the badge and the channel list. `activity:<channel>` reads only
/// ordinary top-level messages in a known non-DM channel; an unknown channel
/// or reader counts as not read, because a DM or mention needs its own mark.
bool messageActionShowsUnread(
  WidgetRef ref,
  ReadStateState readState, {
  required String channelId,
  required TimelineMessage message,
}) {
  final readerPubkey = ref.read(myPubkeyProvider);
  final channels = ref.read(channelsProvider).asData?.value;
  final channel = channels?.where((c) => c.id == channelId).firstOrNull;
  final channelCatchUp =
      readerPubkey != null &&
      channel != null &&
      readByChannelCatchUp(
        isDm: channel.isDm,
        isReply: message.parentId != null,
        highPriority: isHighPriorityEvent(message.tags, readerPubkey),
      );
  return isMessageUnread(
    readState,
    channelId: channelId,
    messageId: message.id,
    createdAt: message.createdAt,
    threadRootId: message.rootId,
    channelCatchUp: channelCatchUp,
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
  final String? currentPubkey;

  const _MarkReadUnreadTile({
    required this.message,
    required this.channelId,
    required this.currentPubkey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final readState = ref.watch(readStateProvider);
    if (!readState.isReady) return const SizedBox.shrink();

    final unread = messageActionShowsUnread(
      ref,
      readState,
      channelId: channelId,
      message: message,
    );

    return ListTile(
      leading: Icon(unread ? BuzzIcons.mailCheck : BuzzIcons.mailOpen),
      title: Text(unread ? 'Mark read' : 'Mark unread'),
      onTap: () {
        if (!_selectMessageAction(context)) return;
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
      leading: Icon(following ? BuzzIcons.bellOff : BuzzIcons.bellRing),
      title: Text(following ? 'Unfollow thread' : 'Follow thread'),
      onTap: () {
        if (!_selectMessageAction(context)) return;
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

class _MessageQuickActionsRow extends ConsumerWidget {
  final TimelineMessage message;
  final String channelId;
  final List<TimelineMessage>? allMessages;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  /// The long-pressed message's page context — survives the sheet pop, used
  /// for the thread push and the copy-link snackbar.
  final BuildContext pageContext;

  const _MessageQuickActionsRow({
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
        _MessageQuickActionTile(
          icon: BuzzIcons.messageSquareReply,
          label: 'Reply',
          onTap: () {
            if (!_selectMessageAction(context)) return;
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
      _MessageQuickActionTile(
        icon: BuzzIcons.link2,
        label: 'Copy link',
        onTap: () {
          if (!_selectMessageAction(context)) return;
          copyToClipboard(
            pageContext,
            messageLinkFor(message: message, channelId: channelId),
            message: 'Message link copied',
          );
        },
      ),
      if (canRemind)
        _MessageQuickActionTile(
          icon: BuzzIcons.clock,
          label: 'Remind me',
          onTap: () {
            final rootContext = Navigator.of(
              context,
              rootNavigator: true,
            ).context;
            if (!_selectMessageAction(context)) return;
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

    return IntrinsicHeight(
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var index = 0; index < tiles.length; index++) ...[
            Expanded(child: tiles[index]),
            if (index < tiles.length - 1) const SizedBox(width: Grid.twelve),
          ],
        ],
      ),
    );
  }
}

class _MessageQuickActionTile extends StatelessWidget {
  final IconData icon;
  final String label;
  final VoidCallback onTap;

  const _MessageQuickActionTile({
    required this.icon,
    required this.label,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    child: Material(
      color: context.colors.surfaceContainerHighest,
      borderRadius: BorderRadius.circular(Radii.dialog),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: () {
          unawaited(HapticFeedback.lightImpact());
          onTap();
        },
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 68 + (Grid.xxs * 2)),
          child: Padding(
            padding: const EdgeInsets.all(Grid.xxs),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(icon, size: 22, color: context.colors.onSurface),
                const SizedBox(height: Grid.xxs),
                Text(
                  label,
                  textAlign: TextAlign.center,
                  style: context.textTheme.labelMedium?.copyWith(
                    color: context.colors.onSurface,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

/// Renders a quick-reaction value: a Unicode glyph as text, a `:shortcode:` as
/// its custom-emoji image.
class _QuickReactionGlyph extends StatelessWidget {
  final String value;
  final Map<String, CustomEmoji> customByShortcode;

  const _QuickReactionGlyph({
    required this.value,
    required this.customByShortcode,
  });

  @override
  Widget build(BuildContext context) {
    if (value.startsWith(':') && value.endsWith(':')) {
      final shortcode = value.substring(1, value.length - 1).toLowerCase();
      final custom = customByShortcode[shortcode];
      if (custom != null) {
        return CustomEmojiImage(
          shortcode: custom.shortcode,
          url: custom.url,
          size: 24,
        );
      }
    }
    return NativeEmojiGlyph(emoji: value, size: 24);
  }
}

/// Circular filled tap target shared by the quick-reaction emojis and the
/// "+" emoji-picker tile — one treatment, one place to change it.
class _QuickReactionCircle extends StatelessWidget {
  final VoidCallback onTap;
  final Widget child;
  final double size;
  final String label;
  final bool? selected;

  const _QuickReactionCircle({
    required this.onTap,
    required this.child,
    required this.size,
    required this.label,
    this.selected,
  });

  @override
  Widget build(BuildContext context) {
    return Semantics(
      button: true,
      label: label,
      selected: selected,
      excludeSemantics: true,
      onTap: onTap,
      child: GestureDetector(
        excludeFromSemantics: true,
        onTap: () {
          unawaited(HapticFeedback.lightImpact());
          onTap();
        },
        child: Container(
          width: size,
          height: size,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: selected == true
                ? context.colors.primary.withValues(alpha: 0.10)
                : context.colors.surfaceContainerHighest,
            border: selected == true
                ? Border.all(
                    color: context.colors.primary.withValues(alpha: 0.40),
                  )
                : null,
            shape: BoxShape.circle,
          ),
          child: child,
        ),
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
  showBuzzModalBottomSheet<void>(
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
  VoidCallback? onDeleted,
}) {
  showBuzzDialog<void>(
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
          onPressed: () async {
            Navigator.of(dialogContext).pop();
            final messenger = ScaffoldMessenger.of(context);
            try {
              await ref
                  .read(channelActionsProvider)
                  .deleteMessage(channelId: channelId, eventId: messageId);
              onDeleted?.call();
            } catch (error) {
              messenger.showSnackBar(
                SnackBar(content: Text('Failed to delete message: $error')),
              );
            }
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
