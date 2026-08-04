import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/mentions/agent_identity_provider.dart';
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
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/read_state/read_state_time.dart';

/// Opens the mobile channel actions sheet and returns whether its parent page
/// should close after a successful lifecycle action.
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

/// Mobile action sheet for channel-level read, organization, and lifecycle
/// operations.
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
        !channel.isDm &&
        ref.watch(channelStarsProvider).store.channels[channel.id]?.starred ==
            true;
    final membersAsync = channel.isDm
        ? const AsyncValue<List<ChannelMember>>.data([])
        : ref.watch(channelMembersProvider(channel.id));
    final agentOwnersAsync = channel.isDm
        ? const AsyncValue<Map<String, String>>.data({})
        : ref.watch(agentOwnersProvider);
    final currentPubkey = ref.watch(currentPubkeyProvider)?.toLowerCase();
    final currentMember = membersAsync.value?.cast<ChannelMember?>().firstWhere(
      (member) => member?.pubkey.toLowerCase() == currentPubkey,
      orElse: () => null,
    );
    final ownsOwnerAgent =
        currentPubkey != null &&
        membersAsync.value?.any(
              (member) =>
                  member.isOwner &&
                  agentOwnersAsync.value?[member.pubkey.toLowerCase()]
                          ?.toLowerCase() ==
                      currentPubkey,
            ) ==
            true;
    final canManageLifecycle =
        currentMember?.isElevated == true || ownsOwnerAgent;
    final canArchive = !channel.isArchived && canManageLifecycle;
    final canUnarchive = channel.isArchived && canManageLifecycle;
    final canDelete =
        !channel.isArchived &&
        (currentMember?.isOwner == true || ownsOwnerAgent);
    final lifecycleCapabilitiesLoading =
        membersAsync.isLoading || agentOwnersAsync.isLoading;
    final lifecycleCapabilitiesUnavailable =
        membersAsync.hasError || agentOwnersAsync.hasError;

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
            if (!channel.isDm) ...[
              _ChannelQuickActionsRow(
                isStarred: isStarred,
                isUnread: isUnread,
                onToggleStar: () {
                  close();
                  final notifier = ref.read(channelStarsProvider.notifier);
                  isStarred
                      ? notifier.unstarChannel(channel.id)
                      : notifier.starChannel(channel.id);
                },
                onToggleRead: () {
                  close();
                  final timestamp = dateTimeToUnixSeconds(
                    channel.lastMessageAt,
                  );
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
              const SizedBox(height: Grid.xs),
            ],
            if (!channel.isDm)
              ListTile(
                leading: const Icon(LucideIcons.folderInput),
                title: const Text('Move to section…'),
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
                leading: const Icon(LucideIcons.settings),
                title: const Text('Manage channel'),
                onTap: () async {
                  final shouldClose = await showModalBottomSheet<bool>(
                    context: context,
                    isScrollControlled: true,
                    showDragHandle: true,
                    constraints: BoxConstraints(
                      maxWidth: 640,
                      maxHeight: MediaQuery.sizeOf(context).height * 0.9,
                    ),
                    builder: (_) => ManageChannelSheet(channel: channel),
                  );
                  if (shouldClose == true && context.mounted) {
                    Navigator.of(context).pop(true);
                  }
                },
              ),
            ListTile(
              leading: const Icon(LucideIcons.copy),
              title: const Text('Copy channel name'),
              onTap: () {
                close();
                copyToClipboard(
                  context,
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
                  context,
                  channel.id,
                  message: 'Channel ID copied to clipboard',
                );
              },
            ),
            if (!channel.isDm) ...[
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
              if (lifecycleCapabilitiesLoading)
                const ListTile(
                  enabled: false,
                  leading: BuzzLoadingIndicator(
                    size: 20,
                    semanticLabel: 'Loading channel actions',
                  ),
                  title: Text('Loading channel actions…'),
                )
              else if (lifecycleCapabilitiesUnavailable)
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
                if (canUnarchive)
                  _ActionTile(
                    icon: LucideIcons.archiveRestore,
                    label: 'Unarchive channel',
                    onTap: () => _confirmAndRun(
                      context,
                      ref,
                      title: 'Unarchive #${channel.name}?',
                      body: 'The channel will become active again.',
                      confirmLabel: 'Unarchive',
                      action: () => ref
                          .read(channelActionsProvider)
                          .unarchiveChannel(channel.id),
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

class _ChannelQuickActionsRow extends StatelessWidget {
  const _ChannelQuickActionsRow({
    required this.isStarred,
    required this.isUnread,
    required this.onToggleStar,
    required this.onToggleRead,
  });

  final bool isStarred;
  final bool isUnread;
  final VoidCallback onToggleStar;
  final VoidCallback onToggleRead;

  @override
  Widget build(BuildContext context) => Row(
    mainAxisAlignment: MainAxisAlignment.spaceEvenly,
    children: [
      _ChannelQuickAction(
        icon: isStarred ? LucideIcons.starOff : LucideIcons.star,
        label: isStarred ? 'Unstar' : 'Star',
        onTap: onToggleStar,
      ),
      _ChannelQuickAction(
        icon: isUnread ? LucideIcons.checkCheck : LucideIcons.circleDot,
        label: isUnread ? 'Mark Read' : 'Mark Unread',
        onTap: onToggleRead,
      ),
    ],
  );
}

class _ChannelQuickAction extends StatelessWidget {
  const _ChannelQuickAction({
    required this.icon,
    required this.label,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => GestureDetector(
    onTap: onTap,
    behavior: HitTestBehavior.opaque,
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
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
