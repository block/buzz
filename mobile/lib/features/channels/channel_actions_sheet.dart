import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/sheet_divider.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channel_mutes/channel_mutes_provider.dart';
import 'channel_sections/channel_sections_provider.dart';
import 'channel_stars/channel_stars_provider.dart';
import 'channels_provider.dart';
import 'manage_channel_sheet.dart';
import 'read_state/read_state_provider.dart';
import 'read_state/read_state_time.dart';

Future<bool?> showChannelActionsSheet({
  required BuildContext context,
  required Channel channel,
  required bool isUnread,
  VoidCallback? onMarkRead,
  String? sectionId,
}) => showModalBottomSheet<bool>(
  context: context,
  isScrollControlled: true,
  showDragHandle: true,
  constraints: BoxConstraints(
    maxWidth: 640,
    maxHeight: MediaQuery.sizeOf(context).height * 0.7,
  ),
  builder: (_) => ChannelActionsSheet(
    channel: channel,
    isUnread: isUnread,
    onMarkRead: onMarkRead,
    sectionId: sectionId,
  ),
);

class ChannelActionsSheet extends ConsumerWidget {
  const ChannelActionsSheet({
    super.key,
    required this.channel,
    required this.isUnread,
    this.onMarkRead,
    this.sectionId,
  });

  final Channel channel;
  final bool isUnread;
  final VoidCallback? onMarkRead;
  final String? sectionId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final isMuted =
        ref.watch(channelMutesProvider).store.channels[channel.id]?.muted ==
        true;
    final isStarred =
        ref.watch(channelStarsProvider).store.channels[channel.id]?.starred ==
        true;
    final membersAsync = channel.isDm
        ? const AsyncValue<List<ChannelMember>>.data([])
        : ref.watch(channelMembersProvider(channel.id));
    final currentPubkey = ref.watch(currentPubkeyProvider)?.toLowerCase();
    final currentMember = membersAsync.value?.cast<ChannelMember?>().firstWhere(
      (member) => member?.pubkey.toLowerCase() == currentPubkey,
      orElse: () => null,
    );
    final canArchive = currentMember?.isElevated == true && !channel.isArchived;
    final canDelete = currentMember?.isOwner == true;

    void close() => Navigator.of(context).pop();

    return SafeArea(
      top: false,
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          0,
          Grid.gutter,
          Grid.xs,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(LucideIcons.copy),
              title: const Text('Copy channel name'),
              onTap: () {
                close();
                copyToClipboard(
                  Navigator.of(context, rootNavigator: true).context,
                  channel.name,
                  message: 'Channel name copied to clipboard',
                );
              },
            ),
            ListTile(
              leading: const Icon(LucideIcons.hash),
              title: const Text('Copy channel ID'),
              onTap: () {
                close();
                copyToClipboard(
                  Navigator.of(context, rootNavigator: true).context,
                  channel.id,
                  message: 'Channel ID copied to clipboard',
                );
              },
            ),
            if (!channel.isDm)
              ListTile(
                leading: const Icon(LucideIcons.folderInput),
                title: const Text('Move to section'),
                onTap: () async {
                  final pageContext = Navigator.of(
                    context,
                    rootNavigator: true,
                  ).context;
                  close();
                  await _showMoveSectionSheet(
                    pageContext,
                    ref,
                    channel: channel,
                    sectionId: sectionId,
                  );
                },
              ),
            const SheetDivider(),
            ListTile(
              leading: Icon(
                isUnread ? LucideIcons.checkCheck : LucideIcons.circleDot,
              ),
              title: Text(isUnread ? 'Mark as read' : 'Mark as unread'),
              onTap: () {
                close();
                final timestamp = dateTimeToUnixSeconds(channel.lastMessageAt);
                if (isUnread) {
                  onMarkRead?.call();
                  if (timestamp != null) {
                    ref
                        .read(readStateProvider.notifier)
                        .markContextRead(
                          channel.id,
                          timestamp,
                          clearForcedMessages: true,
                        );
                    ref
                        .read(channelsProvider.notifier)
                        .clearObservedUnreadCoveredByRead(
                          channel.id,
                          timestamp,
                        );
                  }
                } else {
                  ref
                      .read(readStateProvider.notifier)
                      .markContextUnread(channel.id, channelId: channel.id);
                }
              },
            ),
            ListTile(
              leading: Icon(isMuted ? LucideIcons.bell : LucideIcons.bellOff),
              title: Text(isMuted ? 'Unmute channel' : 'Mute channel'),
              onTap: () {
                close();
                final notifier = ref.read(channelMutesProvider.notifier);
                isMuted
                    ? notifier.unmuteChannel(channel.id)
                    : notifier.muteChannel(channel.id);
              },
            ),
            if (!channel.isDm)
              ListTile(
                leading: Icon(
                  isStarred ? LucideIcons.starOff : LucideIcons.star,
                ),
                title: Text(isStarred ? 'Unstar channel' : 'Star channel'),
                onTap: () {
                  close();
                  final notifier = ref.read(channelStarsProvider.notifier);
                  isStarred
                      ? notifier.unstarChannel(channel.id)
                      : notifier.starChannel(channel.id);
                },
              ),
            if (!channel.isDm) ...[
              ListTile(
                leading: const Icon(LucideIcons.settings),
                title: const Text('Manage channel'),
                onTap: () async {
                  final pageContext = Navigator.of(
                    context,
                    rootNavigator: true,
                  ).context;
                  close();
                  final shouldClose = await showModalBottomSheet<bool>(
                    context: pageContext,
                    isScrollControlled: true,
                    showDragHandle: true,
                    constraints: BoxConstraints(
                      maxWidth: 640,
                      maxHeight: MediaQuery.sizeOf(pageContext).height * 0.9,
                    ),
                    builder: (_) => ManageChannelSheet(channel: channel),
                  );
                  if (shouldClose == true && pageContext.mounted) {
                    Navigator.of(pageContext).pop(true);
                  }
                },
              ),
              const SheetDivider(),
              if (channel.isMember && !channel.isArchived)
                _ActionTile(
                  icon: LucideIcons.logOut,
                  label: 'Leave channel',
                  destructive: true,
                  onTap: () => _confirmAndRun(
                    context,
                    ref,
                    title: 'Leave #${channel.name}?',
                    body: 'You’ll stop receiving messages from this channel.',
                    confirmLabel: 'Leave',
                    action: () => ref
                        .read(channelActionsProvider)
                        .leaveChannel(channel.id),
                  ),
                ),
              if (membersAsync.isLoading)
                const ListTile(
                  enabled: false,
                  leading: BuzzLoadingIndicator(
                    size: 20,
                    semanticLabel: 'Loading channel actions',
                  ),
                  title: Text('Loading channel actions…'),
                )
              else if (membersAsync.hasError)
                const ListTile(
                  enabled: false,
                  leading: Icon(LucideIcons.triangleAlert),
                  title: Text('Channel actions unavailable'),
                )
              else ...[
                if (canArchive)
                  _ActionTile(
                    icon: LucideIcons.archive,
                    label: 'Archive channel',
                    onTap: () => _confirmAndRun(
                      context,
                      ref,
                      title: 'Archive #${channel.name}?',
                      body: 'The channel will become read-only.',
                      confirmLabel: 'Archive',
                      action: () => ref
                          .read(channelActionsProvider)
                          .archiveChannel(channel.id),
                    ),
                  ),
                if (canDelete)
                  _ActionTile(
                    icon: LucideIcons.trash2,
                    label: 'Delete channel',
                    destructive: true,
                    onTap: () => _confirmAndRun(
                      context,
                      ref,
                      title: 'Delete #${channel.name}?',
                      body:
                          'This permanently deletes the channel and cannot be undone.',
                      confirmLabel: 'Delete',
                      action: () => ref
                          .read(channelActionsProvider)
                          .deleteChannel(channel.id),
                    ),
                  ),
              ],
            ],
          ],
        ),
      ),
    );
  }
}

class _ActionTile extends StatelessWidget {
  const _ActionTile({
    required this.icon,
    required this.label,
    required this.onTap,
    this.destructive = false,
  });

  final IconData icon;
  final String label;
  final VoidCallback onTap;
  final bool destructive;

  @override
  Widget build(BuildContext context) => ListTile(
    leading: Icon(icon, color: destructive ? context.colors.error : null),
    title: Text(
      label,
      style: destructive ? TextStyle(color: context.colors.error) : null,
    ),
    onTap: onTap,
  );
}

Future<void> _confirmAndRun(
  BuildContext sheetContext,
  WidgetRef ref, {
  required String title,
  required String body,
  required String confirmLabel,
  required Future<void> Function() action,
}) async {
  final pageContext = Navigator.of(sheetContext, rootNavigator: true).context;
  final confirmed = await showDialog<bool>(
    context: pageContext,
    builder: (dialogContext) => AlertDialog(
      title: Text(title),
      content: Text(body),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(dialogContext).pop(true),
          child: Text(confirmLabel),
        ),
      ],
    ),
  );
  if (confirmed != true) return;
  try {
    await action();
    if (pageContext.mounted) Navigator.of(pageContext).pop(true);
  } catch (error) {
    if (!pageContext.mounted) return;
    ScaffoldMessenger.of(pageContext).showSnackBar(
      SnackBar(
        content: Text(
          'Couldn’t ${confirmLabel.toLowerCase()} channel. Try again.',
        ),
      ),
    );
  }
}

Future<void> _showMoveSectionSheet(
  BuildContext context,
  WidgetRef ref, {
  required Channel channel,
  required String? sectionId,
}) async {
  final sections = [...ref.read(channelSectionsProvider).store.sections]
    ..sort((a, b) => a.order.compareTo(b.order));
  await showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (sheetContext) => SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          0,
          Grid.gutter,
          Grid.xs,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final section in sections)
              ListTile(
                leading: const Icon(LucideIcons.folder),
                title: Text(section.name),
                trailing: sectionId == section.id
                    ? Icon(
                        LucideIcons.check,
                        color: sheetContext.colors.primary,
                      )
                    : null,
                onTap: () {
                  Navigator.of(sheetContext).pop();
                  ref
                      .read(channelSectionsProvider.notifier)
                      .assignChannel(channel.id, section.id);
                },
              ),
            ListTile(
              leading: const Icon(LucideIcons.folderPlus),
              title: const Text('New section…'),
              onTap: () async {
                Navigator.of(sheetContext).pop();
                final name = await _showSectionNameDialog(context);
                if (name == null || name.isEmpty) return;
                final notifier = ref.read(channelSectionsProvider.notifier);
                notifier.createSection(name);
                final created = ref
                    .read(channelSectionsProvider)
                    .store
                    .sections
                    .where((section) => section.name == name.trim())
                    .lastOrNull;
                if (created != null) {
                  notifier.assignChannel(channel.id, created.id);
                }
              },
            ),
            if (sectionId != null)
              ListTile(
                leading: const Icon(LucideIcons.folderMinus),
                title: const Text('Remove from section'),
                onTap: () {
                  Navigator.of(sheetContext).pop();
                  ref
                      .read(channelSectionsProvider.notifier)
                      .unassignChannel(channel.id);
                },
              ),
          ],
        ),
      ),
    ),
  );
}

Future<String?> _showSectionNameDialog(BuildContext context) async {
  final controller = TextEditingController();
  final result = await showDialog<String>(
    context: context,
    builder: (dialogContext) => AlertDialog(
      title: const Text('New Section'),
      content: TextField(controller: controller, autofocus: true),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () =>
              Navigator.of(dialogContext).pop(controller.text.trim()),
          child: const Text('Create'),
        ),
      ],
    ),
  );
  controller.dispose();
  return result;
}
