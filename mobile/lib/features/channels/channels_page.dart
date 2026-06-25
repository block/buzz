import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/auth/auth.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../profile/profile_avatar.dart';
import '../profile/profile_provider.dart';
import '../settings/settings_page.dart';
import '../profile/presence_cache_provider.dart';
import '../profile/user_cache_provider.dart';
import '../pairing/pairing_page.dart';
import '../pairing/pairing_provider.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channel_management_provider.dart';
import 'channel_mutes/channel_mutes_provider.dart';
import 'channel_sections/channel_sections_provider.dart';
import 'channel_sections/channel_sections_storage.dart';
import 'channel_stars/channel_stars_provider.dart';
import 'channels_provider.dart';
import 'read_state/deferred_read_state_update.dart';
import 'read_state/read_state_provider.dart';
import 'read_state/read_state_time.dart';

enum _QuickAction { createChannel, createForum, newDm }

/// Height of the [_ConnectionBanner]: vertical padding (Grid.quarter + 2) × 2
/// plus the ~16px row content (12px spinner / labelSmall text).
const double _kBannerHeight = 24.0;

bool _isUnread(Channel channel, ReadStateState readState) {
  if (readState.locallyForcedChannelIds.contains(channel.id)) {
    return true;
  }

  final lastMessageAt = dateTimeToUnixSeconds(channel.lastMessageAt);
  if (lastMessageAt == null) {
    return false;
  }

  final readAt = readState.effectiveTimestamp(channel.id);
  return readAt == null || lastMessageAt > readAt;
}

class ChannelsPage extends HookConsumerWidget {
  const ChannelsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channelsAsync = ref.watch(channelsProvider);
    final sessionState = ref.watch(relaySessionProvider);
    final currentPubkey = ref
        .watch(profileProvider)
        .whenData((value) => value?.pubkey)
        .value;

    // Cache the last successfully loaded channels so the UI never flashes
    // back to a loading state when the provider rebuilds (e.g. reconnect).
    // Clear the cache on workspace switch so we show a full loader instead of
    // stale channels from the previous workspace. unwrapPrevious() ensures the
    // selector sees null during loading (not the previous workspace's ID).
    final activeWorkspaceId = ref.watch(
      activeWorkspaceProvider.select((v) => v.unwrapPrevious().value?.id),
    );
    final cachedChannels = useRef<List<Channel>?>(null);
    final lastWorkspaceId = useRef<String?>(null);
    if (lastWorkspaceId.value != activeWorkspaceId) {
      cachedChannels.value = null;
      lastWorkspaceId.value = activeWorkspaceId;
    }
    if (channelsAsync.asData?.value case final data?) {
      cachedChannels.value = data;
    }
    final channels = cachedChannels.value;

    Future<void> openChannel(Channel channel) async {
      if (!context.mounted) return;
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(channel: channel),
        ),
      );
    }

    Future<void> openQuickActions() async {
      final action = await showModalBottomSheet<_QuickAction>(
        context: context,
        showDragHandle: true,
        builder: (_) => const _QuickActionsSheet(),
      );

      if (!context.mounted || action == null) {
        return;
      }

      switch (action) {
        case _QuickAction.createChannel:
        case _QuickAction.createForum:
          final created = await showModalBottomSheet<Channel>(
            context: context,
            isScrollControlled: true,
            showDragHandle: true,
            builder: (_) => _CreateChannelSheet(
              channelType: action == _QuickAction.createForum
                  ? 'forum'
                  : 'stream',
            ),
          );
          if (created != null && context.mounted) {
            await openChannel(created);
          }
        case _QuickAction.newDm:
          final opened = await showModalBottomSheet<Channel>(
            context: context,
            isScrollControlled: true,
            showDragHandle: true,
            builder: (_) =>
                _NewDirectMessageSheet(currentPubkey: currentPubkey),
          );
          if (opened != null && context.mounted) {
            await openChannel(opened);
          }
      }
    }

    // Defer the error view to absorb transient AsyncError frames caused by
    // the relay session cancelling in-flight history fetches on disconnect/
    // reconnect (relay_session.dart `_cancelAllHistory`). If the error clears
    // (channels populate or the next _fetch succeeds) within the grace
    // window, we never render the error UI.
    final showError = useState(false);
    final hasError = channelsAsync.hasError && channels == null;
    useEffect(() {
      if (!hasError) {
        showError.value = false;
        return null;
      }
      final timer = Timer(const Duration(seconds: 2), () {
        showError.value = true;
      });
      return timer.cancel;
    }, [hasError]);

    return FrostedScaffold(
      appBar: FrostedAppBar(
        leading: _WorkspaceIndicator(
          onTap: () => showModalBottomSheet<void>(
            context: context,
            showDragHandle: true,
            builder: (_) => const _WorkspaceSwitcherSheet(),
          ),
        ),
        title: const SizedBox.shrink(),
        actions: [
          ProfileAvatar(
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const SettingsPage()),
            ),
          ),
          const SizedBox(width: Grid.xs),
        ],
      ),
      floatingActionButton: FloatingActionButton(
        heroTag: 'channels-fab',
        onPressed: openQuickActions,
        tooltip: 'Create or start conversation',
        shape: const CircleBorder(),
        child: const Icon(LucideIcons.plus),
      ),
      body: _ChannelsBody(
        channels: channels,
        channelsAsync: channelsAsync,
        showError: showError.value,
        sessionStatus: sessionState.status,
        currentPubkey: currentPubkey,
        onRefresh: () => ref.read(channelsProvider.notifier).refresh(),
        onSelectChannel: openChannel,
      ),
    );
  }
}

class _ChannelsBody extends StatelessWidget {
  final List<Channel>? channels;
  final AsyncValue<List<Channel>> channelsAsync;
  final bool showError;
  final SessionStatus sessionStatus;
  final String? currentPubkey;
  final Future<void> Function() onRefresh;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _ChannelsBody({
    required this.channels,
    required this.channelsAsync,
    required this.showError,
    required this.sessionStatus,
    required this.currentPubkey,
    required this.onRefresh,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context) {
    final barHeight = frostedAppBarHeight(context);

    if (channels != null) {
      return Stack(
        children: [
          RefreshIndicator(
            edgeOffset: barHeight,
            onRefresh: onRefresh,
            child: CustomScrollView(
              slivers: [
                SliverToBoxAdapter(child: SizedBox(height: barHeight)),
                // Extra space for the connection banner when visible.
                if (sessionStatus != SessionStatus.connected &&
                    sessionStatus != SessionStatus.disconnected)
                  const SliverToBoxAdapter(
                    child: SizedBox(height: _kBannerHeight),
                  ),
                _SliverChannelsList(
                  channels: channels!,
                  currentPubkey: currentPubkey,
                  onSelectChannel: onSelectChannel,
                ),
              ],
            ),
          ),
          Positioned(
            top: barHeight,
            left: 0,
            right: 0,
            child: _ConnectionBanner(status: sessionStatus),
          ),
        ],
      );
    }

    // The error view is gated on a grace timer in the parent — see the
    // useEffect in ChannelsPage. While the grace window is in flight we fall
    // through to the connection banner so transient relay-cancellation errors
    // don't flash the error UI.
    if (showError && channelsAsync.hasError) {
      return Padding(
        padding: EdgeInsets.only(top: barHeight),
        child: _ErrorView(error: channelsAsync.error!, onRetry: onRefresh),
      );
    }

    return Padding(
      padding: EdgeInsets.only(top: barHeight),
      child: _ConnectionBanner(
        status: sessionStatus == SessionStatus.connected
            ? SessionStatus.connecting
            : sessionStatus,
      ),
    );
  }
}

class _SliverChannelsList extends HookConsumerWidget {
  final List<Channel> channels;
  final String? currentPubkey;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _SliverChannelsList({
    required this.channels,
    required this.currentPubkey,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final readState = ref.watch(readStateProvider);
    final sectionsState = ref.watch(channelSectionsProvider);
    final mutesState = ref.watch(channelMutesProvider);
    final mutedChannelIds = {
      for (final entry in mutesState.store.channels.entries)
        if (entry.value.muted) entry.key,
    };
    final starsState = ref.watch(channelStarsProvider);
    final starredChannelIds = {
      for (final entry in starsState.store.channels.entries)
        if (entry.value.starred) entry.key,
    };
    final visibleChannels = channels
        .where((channel) => channel.isMember && !channel.isArchived)
        .toList();
    final streamChannels = visibleChannels
        .where((channel) => channel.isStream)
        .toList();
    final forumChannels = visibleChannels
        .where((channel) => channel.isForum)
        .toList();
    final dmChannels = visibleChannels
        .where((channel) => channel.isDm)
        .toList();

    final starredExpanded = useState(true);
    final channelsExpanded = useState(true);
    final forumsExpanded = useState(true);
    final dmsExpanded = useState(true);
    final initialSeedComplete = useState(false);
    final seededPubkey = useRef<String?>(null);
    final seedCompleteForPubkey =
        seededPubkey.value == readState.pubkey && initialSeedComplete.value;

    useEffect(() {
      if (!readState.isReady) {
        return null;
      }

      return deferReadStateUpdate(context, () {
        if (seededPubkey.value != readState.pubkey) {
          seededPubkey.value = readState.pubkey;
          initialSeedComplete.value = false;
        }

        if (initialSeedComplete.value) {
          return;
        }

        final notifier = ref.read(readStateProvider.notifier);
        for (final channel in visibleChannels) {
          if (readState.effectiveTimestamp(channel.id) != null) {
            continue;
          }

          final lastMessageAt = dateTimeToUnixSeconds(channel.lastMessageAt);
          if (lastMessageAt != null) {
            notifier.seedContextRead(channel.id, lastMessageAt);
          }
        }
        initialSeedComplete.value = true;
      });
    }, [readState.isReady, readState.pubkey, visibleChannels]);

    final unreadChannelIds = readState.isReady
        ? {
            for (final channel in visibleChannels)
              if ((seedCompleteForPubkey ||
                      readState.effectiveTimestamp(channel.id) != null) &&
                  _isUnread(channel, readState) &&
                  !mutedChannelIds.contains(channel.id))
                channel.id,
          }
        : const <String>{};

    // Build sorted user-defined sections and compute which stream channels
    // belong to each section. Channels not assigned to any valid section fall
    // through to the built-in "Channels" list.
    final userSections = sectionsState.store.sections.toList()
      ..sort((a, b) => a.order.compareTo(b.order));
    final sectionAssignments = sectionsState.store.assignments;
    final validSectionIds = {for (final s in userSections) s.id};
    final assignedChannelIds = {
      for (final entry in sectionAssignments.entries)
        if (validSectionIds.contains(entry.value)) entry.key,
    };
    // Starred is exclusive: a starred channel lives only in the Starred section,
    // not in its custom section or the default Channels list.
    final starredStreamChannels = streamChannels
        .where((c) => starredChannelIds.contains(c.id))
        .toList();
    final ungroupedStreamChannels = streamChannels
        .where(
          (c) =>
              !assignedChannelIds.contains(c.id) &&
              !starredChannelIds.contains(c.id),
        )
        .toList();

    final sectionExpandedStates = useState<Map<String, bool>>({});

    bool sectionExpanded(String sectionId) =>
        sectionExpandedStates.value[sectionId] ?? true;

    void toggleSection(String sectionId) {
      sectionExpandedStates.value = {
        ...sectionExpandedStates.value,
        sectionId: !sectionExpanded(sectionId),
      };
    }

    return SliverPadding(
      padding: const EdgeInsets.only(top: Grid.xxs, bottom: 80),
      sliver: SliverList.list(
        children: [
          if (visibleChannels.isEmpty)
            const _EmptyState()
          else ...[
            // Starred channels (exclusive — pinned above all sections).
            if (starredStreamChannels.isNotEmpty)
              _ChannelSection(
                title: 'Starred',
                icon: LucideIcons.star,
                expanded: starredExpanded.value,
                onToggle: () => starredExpanded.value = !starredExpanded.value,
                channels: starredStreamChannels,
                unreadChannelIds: unreadChannelIds,
                mutedChannelIds: mutedChannelIds,
                currentPubkey: currentPubkey,
                emptyLabel: '',
                onSelectChannel: onSelectChannel,
              ),
            // User-defined sections for stream channels, in user-defined order.
            for (final section in userSections)
              _CustomChannelSection(
                section: section,
                channels: streamChannels
                    .where(
                      (c) =>
                          sectionAssignments[c.id] == section.id &&
                          !starredChannelIds.contains(c.id),
                    )
                    .toList(),
                unreadChannelIds: unreadChannelIds,
                mutedChannelIds: mutedChannelIds,
                currentPubkey: currentPubkey,
                expanded: sectionExpanded(section.id),
                isFirst: userSections.first.id == section.id,
                isLast: userSections.last.id == section.id,
                onToggle: () => toggleSection(section.id),
                onRename: () async {
                  final name = await showDialog<String>(
                    context: context,
                    builder: (_) => _SectionNameDialog(
                      title: 'Rename Section',
                      confirmLabel: 'Rename',
                      initialValue: section.name,
                    ),
                  );
                  if (name != null && name.isNotEmpty) {
                    ref
                        .read(channelSectionsProvider.notifier)
                        .renameSection(section.id, name);
                  }
                },
                onDelete: () async {
                  final confirmed = await showDialog<bool>(
                    context: context,
                    builder: (_) => AlertDialog(
                      title: Text('Delete "${section.name}"?'),
                      content: const Text(
                        'Channels in this section will move back to the main list.',
                      ),
                      actions: [
                        TextButton(
                          onPressed: () => Navigator.pop(context, false),
                          child: const Text('Cancel'),
                        ),
                        TextButton(
                          onPressed: () => Navigator.pop(context, true),
                          child: Text(
                            'Delete',
                            style: TextStyle(
                              color: Theme.of(context).colorScheme.error,
                            ),
                          ),
                        ),
                      ],
                    ),
                  );
                  if (confirmed == true) {
                    ref
                        .read(channelSectionsProvider.notifier)
                        .deleteSection(section.id);
                  }
                },
                onMoveUp: () => ref
                    .read(channelSectionsProvider.notifier)
                    .moveSectionUp(section.id),
                onMoveDown: () => ref
                    .read(channelSectionsProvider.notifier)
                    .moveSectionDown(section.id),
                onSelectChannel: onSelectChannel,
                onMarkChannelRead: (channel) {
                  final ts = dateTimeToUnixSeconds(channel.lastMessageAt);
                  if (ts != null) {
                    ref
                        .read(readStateProvider.notifier)
                        .markContextRead(channel.id, ts);
                  }
                },
              ),
            _ChannelSection(
              title: 'Channels',
              icon: LucideIcons.hash,
              expanded: channelsExpanded.value,
              onToggle: () => channelsExpanded.value = !channelsExpanded.value,
              channels: ungroupedStreamChannels,
              unreadChannelIds: unreadChannelIds,
              mutedChannelIds: mutedChannelIds,
              currentPubkey: currentPubkey,
              emptyLabel: 'No stream channels yet',
              onSelectChannel: onSelectChannel,
            ),
            _ChannelSection(
              title: 'Forums',
              icon: LucideIcons.messageSquareText,
              expanded: forumsExpanded.value,
              onToggle: () => forumsExpanded.value = !forumsExpanded.value,
              channels: forumChannels,
              unreadChannelIds: unreadChannelIds,
              mutedChannelIds: mutedChannelIds,
              currentPubkey: currentPubkey,
              emptyLabel: 'No forums yet',
              onSelectChannel: onSelectChannel,
            ),
            _ChannelSection(
              title: 'DMs',
              icon: LucideIcons.messagesSquare,
              expanded: dmsExpanded.value,
              onToggle: () => dmsExpanded.value = !dmsExpanded.value,
              channels: dmChannels,
              unreadChannelIds: unreadChannelIds,
              mutedChannelIds: mutedChannelIds,
              currentPubkey: currentPubkey,
              emptyLabel: 'No direct messages yet',
              onSelectChannel: onSelectChannel,
            ),
          ],
        ],
      ),
    );
  }
}

class _CustomChannelSection extends StatelessWidget {
  final ChannelSection section;
  final List<Channel> channels;
  final Set<String> unreadChannelIds;
  final Set<String> mutedChannelIds;
  final String? currentPubkey;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final VoidCallback onToggle;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final VoidCallback onMoveUp;
  final VoidCallback onMoveDown;
  final Future<void> Function(Channel channel) onSelectChannel;
  final void Function(Channel channel) onMarkChannelRead;

  const _CustomChannelSection({
    required this.section,
    required this.channels,
    required this.unreadChannelIds,
    required this.mutedChannelIds,
    required this.currentPubkey,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.onToggle,
    required this.onRename,
    required this.onDelete,
    required this.onMoveUp,
    required this.onMoveDown,
    required this.onSelectChannel,
    required this.onMarkChannelRead,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _CustomSectionHeader(
          section: section,
          expanded: expanded,
          isFirst: isFirst,
          isLast: isLast,
          onToggle: onToggle,
          onRename: onRename,
          onDelete: onDelete,
          onMoveUp: onMoveUp,
          onMoveDown: onMoveDown,
        ),
        if (expanded)
          for (final channel in channels)
            _ChannelTile(
              channel: channel,
              isUnread: unreadChannelIds.contains(channel.id),
              isMuted: mutedChannelIds.contains(channel.id),
              currentPubkey: currentPubkey,
              onTap: () => onSelectChannel(channel),
              onMarkRead: () => onMarkChannelRead(channel),
              sectionId: section.id,
            ),
      ],
    );
  }
}

class _CustomSectionHeader extends StatelessWidget {
  final ChannelSection section;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final VoidCallback onToggle;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final VoidCallback onMoveUp;
  final VoidCallback onMoveDown;

  const _CustomSectionHeader({
    required this.section,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.onToggle,
    required this.onRename,
    required this.onDelete,
    required this.onMoveUp,
    required this.onMoveDown,
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.xs,
          Grid.twelve,
          Grid.xs,
          Grid.half,
        ),
        child: Row(
          children: [
            Icon(
              LucideIcons.folder,
              size: 14,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(width: Grid.half),
            Text(
              section.name.toUpperCase(),
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.8,
              ),
            ),
            const Spacer(),
            GestureDetector(
              onTapUp: (details) async {
                final overlay =
                    Overlay.of(context).context.findRenderObject()!
                        as RenderBox;
                final position = RelativeRect.fromRect(
                  details.globalPosition & Size.zero,
                  Offset.zero & overlay.size,
                );
                final value = await showMenu<String>(
                  context: context,
                  position: position,
                  items: [
                    const PopupMenuItem(value: 'rename', child: Text('Rename')),
                    PopupMenuItem(
                      value: 'move_up',
                      enabled: !isFirst,
                      child: const Text('Move Up'),
                    ),
                    PopupMenuItem(
                      value: 'move_down',
                      enabled: !isLast,
                      child: const Text('Move Down'),
                    ),
                    const PopupMenuItem(value: 'delete', child: Text('Delete')),
                  ],
                );
                switch (value) {
                  case 'rename':
                    onRename();
                  case 'move_up':
                    onMoveUp();
                  case 'move_down':
                    onMoveDown();
                  case 'delete':
                    onDelete();
                }
              },
              child: Icon(
                LucideIcons.ellipsisVertical,
                size: 14,
                color: context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(width: Grid.quarter),
            Icon(
              expanded ? LucideIcons.chevronDown : LucideIcons.chevronRight,
              size: 14,
              color: context.colors.onSurfaceVariant,
            ),
          ],
        ),
      ),
    );
  }
}

class _SectionNameDialog extends HookWidget {
  final String title;
  final String confirmLabel;
  final String initialValue;

  const _SectionNameDialog({
    required this.title,
    required this.confirmLabel,
    this.initialValue = '',
  });

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialValue);

    void confirm() {
      final name = controller.text.trim();
      if (name.isNotEmpty) Navigator.of(context).pop(name);
    }

    return AlertDialog(
      title: Text(title),
      content: TextField(
        controller: controller,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Name'),
        onSubmitted: (_) => confirm(),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(onPressed: confirm, child: Text(confirmLabel)),
      ],
    );
  }
}

class _ChannelSection extends StatelessWidget {
  final String title;
  final IconData icon;
  final bool expanded;
  final VoidCallback onToggle;
  final List<Channel> channels;
  final Set<String> unreadChannelIds;
  final Set<String> mutedChannelIds;
  final String? currentPubkey;
  final String emptyLabel;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _ChannelSection({
    required this.title,
    required this.icon,
    required this.expanded,
    required this.onToggle,
    required this.channels,
    required this.unreadChannelIds,
    required this.mutedChannelIds,
    required this.currentPubkey,
    required this.emptyLabel,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _SectionHeader(
          label: title,
          icon: icon,
          expanded: expanded,
          onToggle: onToggle,
        ),
        if (expanded) ...[
          if (channels.isEmpty)
            Padding(
              padding: const EdgeInsets.only(
                left: Grid.xs + Grid.xxs,
                right: Grid.xs,
                top: Grid.half,
                bottom: Grid.half,
              ),
              child: Text(
                emptyLabel,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            )
          else
            for (final channel in channels)
              _ChannelTile(
                channel: channel,
                isUnread: unreadChannelIds.contains(channel.id),
                isMuted: mutedChannelIds.contains(channel.id),
                currentPubkey: currentPubkey,
                onTap: () => onSelectChannel(channel),
                onMarkRead: null,
                sectionId: null,
              ),
        ],
      ],
    );
  }
}

class _EmptyState extends StatelessWidget {
  const _EmptyState();

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: MediaQuery.sizeOf(context).height * 0.55,
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.messagesSquare,
              size: Grid.xl,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(height: Grid.xs),
            Text(
              'No conversations yet',
              style: context.textTheme.bodyLarge?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SectionHeader extends StatelessWidget {
  final String label;
  final IconData icon;
  final bool expanded;
  final VoidCallback onToggle;

  const _SectionHeader({
    required this.label,
    required this.icon,
    required this.expanded,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.xs,
          Grid.twelve,
          Grid.xs,
          Grid.half,
        ),
        child: Row(
          children: [
            Icon(icon, size: 14, color: context.colors.onSurfaceVariant),
            const SizedBox(width: Grid.half),
            Text(
              label.toUpperCase(),
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.8,
              ),
            ),
            const Spacer(),
            Icon(
              expanded ? LucideIcons.chevronDown : LucideIcons.chevronRight,
              size: 14,
              color: context.colors.onSurfaceVariant,
            ),
          ],
        ),
      ),
    );
  }
}

class _ChannelTile extends ConsumerWidget {
  final Channel channel;
  final bool isUnread;
  final bool isMuted;
  final String? currentPubkey;
  final VoidCallback onTap;

  /// Called when the user requests to mark this channel read (from long-press
  /// actions menu). Null for channels in built-in sections.
  final VoidCallback? onMarkRead;

  /// The user-defined section this channel currently belongs to, or null.
  final String? sectionId;

  const _ChannelTile({
    required this.channel,
    required this.isUnread,
    required this.currentPubkey,
    required this.onTap,
    this.isMuted = false,
    this.onMarkRead,
    this.sectionId,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final hasActivity = channel.lastMessageAt != null;

    return InkWell(
      borderRadius: BorderRadius.circular(Radii.md),
      onTap: onTap,
      onLongPress: () => _showChannelActions(context, ref),
      child: Opacity(
        opacity: isMuted ? 0.5 : 1.0,
        child: Padding(
          padding: const EdgeInsets.only(
            left: Grid.xs + Grid.xxs,
            right: Grid.xs,
            top: Grid.xxs + Grid.quarter,
            bottom: Grid.xxs + Grid.quarter,
          ),
          child: Row(
            children: [
              if (channel.isDm)
                _DmAvatar(channel: channel, currentPubkey: currentPubkey)
              else
                Icon(
                  channelIcon(channel),
                  size: 18,
                  color: hasActivity
                      ? context.colors.onSurface
                      : context.colors.onSurfaceVariant,
                ),
              const SizedBox(width: Grid.xxs),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      channel.displayLabel(currentPubkey: currentPubkey),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: isUnread
                            ? context.colors.onSurface
                            : hasActivity
                            ? context.colors.onSurface
                            : context.colors.onSurfaceVariant,
                        fontWeight: isUnread && !isMuted
                            ? FontWeight.w700
                            : null,
                      ),
                    ),
                  ],
                ),
              ),
              if (isMuted) ...[
                const SizedBox(width: Grid.xxs),
                Icon(
                  LucideIcons.bellOff,
                  size: 12,
                  color: context.colors.onSurfaceVariant,
                ),
              ] else if (isUnread) ...[
                const SizedBox(width: Grid.xxs),
                Container(
                  key: Key('channel-unread-${channel.id}'),
                  width: 8,
                  height: 8,
                  decoration: BoxDecoration(
                    color: context.colors.primary,
                    shape: BoxShape.circle,
                  ),
                ),
              ],
              if (!channel.isMember && !channel.isDm)
                Padding(
                  padding: const EdgeInsets.only(right: Grid.xxs),
                  child: Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.half + 2,
                      vertical: 3,
                    ),
                    decoration: BoxDecoration(
                      color: context.colors.primary.withValues(alpha: 0.1),
                      borderRadius: BorderRadius.circular(Radii.sm),
                    ),
                    child: Text(
                      'Open',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.primary,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ),
              if (channel.isEphemeral) ...[
                const SizedBox(width: Grid.xxs),
                _EphemeralBadge(channel: channel),
              ],
            ],
          ),
        ),
      ),
    );
  }

  void _showChannelActions(BuildContext context, WidgetRef ref) {
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) {
        final sections = ref.read(channelSectionsProvider).store.sections
          ..sort((a, b) => a.order.compareTo(b.order));
        final isStarred =
            ref
                .read(channelStarsProvider)
                .store
                .channels[channel.id]
                ?.starred ==
            true;

        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(Grid.xs, 0, Grid.xs, Grid.xs),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                ListTile(
                  leading: Icon(
                    isStarred ? LucideIcons.starOff : LucideIcons.star,
                  ),
                  title: Text(isStarred ? 'Unstar channel' : 'Star channel'),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    if (isStarred) {
                      ref
                          .read(channelStarsProvider.notifier)
                          .unstarChannel(channel.id);
                    } else {
                      ref
                          .read(channelStarsProvider.notifier)
                          .starChannel(channel.id);
                    }
                  },
                ),
                ListTile(
                  leading: const Icon(LucideIcons.folderInput),
                  title: const Text('Move to section'),
                  onTap: () async {
                    Navigator.of(sheetContext).pop();
                    await _showMoveSectionSheet(context, ref, sections);
                  },
                ),
                ListTile(
                  leading: Icon(
                    isMuted ? LucideIcons.bell : LucideIcons.bellOff,
                  ),
                  title: Text(isMuted ? 'Unmute channel' : 'Mute channel'),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    if (isMuted) {
                      ref
                          .read(channelMutesProvider.notifier)
                          .unmuteChannel(channel.id);
                    } else {
                      ref
                          .read(channelMutesProvider.notifier)
                          .muteChannel(channel.id);
                    }
                  },
                ),
                ListTile(
                  leading: Icon(
                    isUnread ? LucideIcons.checkCheck : LucideIcons.circleDot,
                  ),
                  title: Text(isUnread ? 'Mark as read' : 'Mark as unread'),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    final ts = dateTimeToUnixSeconds(channel.lastMessageAt);
                    if (ts != null) {
                      if (isUnread) {
                        onMarkRead?.call();
                        ref
                            .read(readStateProvider.notifier)
                            .markContextRead(channel.id, ts);
                      } else {
                        ref
                            .read(readStateProvider.notifier)
                            .markContextUnread(channel.id);
                      }
                    }
                  },
                ),
              ],
            ),
          ),
        );
      },
    );
  }

  Future<void> _showMoveSectionSheet(
    BuildContext context,
    WidgetRef ref,
    List<ChannelSection> sections,
  ) async {
    await showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) {
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(Grid.xs, 0, Grid.xs, Grid.xs),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final section in sections)
                  ListTile(
                    leading: Icon(
                      LucideIcons.folder,
                      color: sectionId == section.id
                          ? Theme.of(sheetContext).colorScheme.primary
                          : null,
                    ),
                    title: Text(section.name),
                    trailing: sectionId == section.id
                        ? Icon(
                            LucideIcons.check,
                            color: Theme.of(sheetContext).colorScheme.primary,
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
                    if (!context.mounted) return;
                    final name = await showDialog<String>(
                      context: context,
                      builder: (_) => const _SectionNameDialog(
                        title: 'New Section',
                        confirmLabel: 'Create',
                      ),
                    );
                    if (name != null && name.isNotEmpty) {
                      ref
                          .read(channelSectionsProvider.notifier)
                          .createSection(name);
                      // Assign after create — sections list has been mutated,
                      // re-read to find the new section by name.
                      final newSection = ref
                          .read(channelSectionsProvider)
                          .store
                          .sections
                          .lastWhere(
                            (s) => s.name == name.trim(),
                            orElse: () => const ChannelSection(
                              id: '',
                              name: '',
                              order: -1,
                            ),
                          );
                      if (newSection.id.isNotEmpty) {
                        ref
                            .read(channelSectionsProvider.notifier)
                            .assignChannel(channel.id, newSection.id);
                      }
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
        );
      },
    );
  }
}

class _DmAvatar extends ConsumerWidget {
  final Channel channel;
  final String? currentPubkey;

  const _DmAvatar({required this.channel, required this.currentPubkey});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profiles = ref.watch(userCacheProvider);
    final presenceMap = ref.watch(presenceCacheProvider);
    final normalizedCurrent = currentPubkey?.toLowerCase();

    // Find the other participant's pubkey.
    String? otherPubkey;
    for (final pk in channel.participantPubkeys) {
      if (pk.toLowerCase() != normalizedCurrent) {
        otherPubkey = pk.toLowerCase();
        break;
      }
    }

    final profile = otherPubkey != null ? profiles[otherPubkey] : null;

    // Trigger fetches if not cached yet.
    if (otherPubkey != null) {
      if (profile == null) {
        ref.read(userCacheProvider.notifier).preload([otherPubkey]);
      }
      ref.read(presenceCacheProvider.notifier).track([otherPubkey]);
    }

    final avatarUrl = profile?.avatarUrl;
    final initial =
        profile?.initial ??
        (channel.participants.isNotEmpty
            ? channel.participants.first[0].toUpperCase()
            : '?');
    final presence = otherPubkey != null
        ? (presenceMap[otherPubkey] ?? 'offline')
        : 'offline';

    return SizedBox(
      width: 22,
      height: 22,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          CircleAvatar(
            radius: 10,
            backgroundColor: context.colors.primaryContainer,
            backgroundImage: avatarUrl != null ? NetworkImage(avatarUrl) : null,
            child: avatarUrl == null
                ? Text(
                    initial,
                    style: context.textTheme.labelSmall?.copyWith(
                      fontSize: 9,
                      color: context.colors.onPrimaryContainer,
                      fontWeight: FontWeight.w600,
                    ),
                  )
                : null,
          ),
          Positioned(
            right: -1,
            bottom: -1,
            child: Container(
              width: 9,
              height: 9,
              decoration: BoxDecoration(
                color: _presenceColor(context, presence),
                shape: BoxShape.circle,
                border: Border.all(
                  color: context.theme.scaffoldBackgroundColor,
                  width: 1.5,
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Color _presenceColor(BuildContext context, String presence) {
    return switch (presence) {
      'online' => context.appColors.success,
      'away' => context.appColors.warning,
      _ => context.colors.outline,
    };
  }
}

class _QuickActionsSheet extends StatelessWidget {
  const _QuickActionsSheet();

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(Grid.xs, 0, Grid.xs, Grid.xs),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            ListTile(
              leading: const Icon(LucideIcons.hash),
              title: const Text('Create channel'),
              subtitle: const Text('Start a new stream channel'),
              onTap: () =>
                  Navigator.of(context).pop(_QuickAction.createChannel),
            ),
            ListTile(
              leading: const Icon(LucideIcons.messageSquareText),
              title: const Text('Create forum'),
              subtitle: const Text('Set up a threaded discussion space'),
              onTap: () => Navigator.of(context).pop(_QuickAction.createForum),
            ),
            ListTile(
              leading: const Icon(LucideIcons.messagesSquare),
              title: const Text('New direct message'),
              subtitle: const Text('Open a DM with one or more people'),
              onTap: () => Navigator.of(context).pop(_QuickAction.newDm),
            ),
          ],
        ),
      ),
    );
  }
}

class _CreateChannelSheet extends HookConsumerWidget {
  final String channelType;

  const _CreateChannelSheet({required this.channelType});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final nameController = useTextEditingController();
    final descriptionController = useTextEditingController();
    final visibility = useState('open');
    final isSubmitting = useState(false);
    final errorMessage = useState<String?>(null);

    Future<void> submit() async {
      final name = nameController.text.trim();
      if (name.isEmpty || isSubmitting.value) {
        return;
      }

      isSubmitting.value = true;
      errorMessage.value = null;
      try {
        final created = await ref
            .read(channelActionsProvider)
            .createChannel(
              name: name,
              channelType: channelType,
              visibility: visibility.value,
              description: descriptionController.text.trim(),
            );
        if (context.mounted) {
          Navigator.of(context).pop(created);
        }
      } catch (error) {
        errorMessage.value = error.toString();
      } finally {
        isSubmitting.value = false;
      }
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.xs,
        0,
        Grid.xs,
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: nameController,
              enabled: !isSubmitting.value,
              decoration: InputDecoration(
                labelText: 'Name',
                hintText: channelType == 'forum'
                    ? 'design-discussions'
                    : 'release-notes',
              ),
              textInputAction: TextInputAction.next,
            ),
            const SizedBox(height: Grid.xxs),
            TextField(
              controller: descriptionController,
              enabled: !isSubmitting.value,
              decoration: const InputDecoration(
                labelText: 'Description',
                hintText: 'What this space is for',
              ),
              minLines: 2,
              maxLines: 3,
            ),
            const SizedBox(height: Grid.xxs),
            SwitchListTile(
              title: const Text('Private'),
              contentPadding: EdgeInsets.zero,
              value: visibility.value == 'private',
              onChanged: isSubmitting.value
                  ? null
                  : (on) => visibility.value = on ? 'private' : 'open',
            ),
            if (errorMessage.value case final error?) ...[
              const SizedBox(height: Grid.xxs),
              Text(
                error,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.xs),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                TextButton(
                  onPressed: isSubmitting.value
                      ? null
                      : () => Navigator.of(context).pop(),
                  child: const Text('Cancel'),
                ),
                const SizedBox(width: Grid.half),
                FilledButton(
                  onPressed: isSubmitting.value ? null : submit,
                  child: Text(isSubmitting.value ? 'Creating…' : 'Create'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _NewDirectMessageSheet extends HookConsumerWidget {
  final String? currentPubkey;

  const _NewDirectMessageSheet({required this.currentPubkey});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final queryController = useTextEditingController();
    final query = useState('');
    final debouncedQuery = useState('');
    final selectedUsers = useState<List<DirectoryUser>>([]);
    final isSubmitting = useState(false);
    final submitError = useState<String?>(null);

    useEffect(() {
      final timer = Timer(const Duration(milliseconds: 250), () {
        debouncedQuery.value = query.value.trim();
      });
      return timer.cancel;
    }, [query.value]);

    final searchFuture = useMemoized(() {
      if (debouncedQuery.value.isEmpty || selectedUsers.value.length >= 8) {
        return Future.value(const <DirectoryUser>[]);
      }
      return ref
          .read(channelActionsProvider)
          .searchUsers(debouncedQuery.value, limit: 8);
    }, [debouncedQuery.value, selectedUsers.value.length]);
    final searchResults = useFuture(searchFuture);

    final selectedPubkeys = selectedUsers.value
        .map((user) => user.pubkey.toLowerCase())
        .toSet();
    final availableResults =
        searchResults.data
            ?.where(
              (user) =>
                  !selectedPubkeys.contains(user.pubkey.toLowerCase()) &&
                  user.pubkey.toLowerCase() != currentPubkey?.toLowerCase(),
            )
            .toList() ??
        const <DirectoryUser>[];
    final canSubmit = !isSubmitting.value && selectedUsers.value.isNotEmpty;

    Future<void> submit() async {
      if (selectedUsers.value.isEmpty || isSubmitting.value) {
        return;
      }

      isSubmitting.value = true;
      submitError.value = null;
      try {
        final channel = await ref
            .read(channelActionsProvider)
            .openDm(
              pubkeys: selectedUsers.value.map((user) => user.pubkey).toList(),
            );
        if (context.mounted) {
          Navigator.of(context).pop(channel);
        }
      } catch (error) {
        submitError.value = error.toString();
      } finally {
        isSubmitting.value = false;
      }
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.xs,
        0,
        Grid.xs,
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: queryController,
              decoration: const InputDecoration(
                prefixIcon: Icon(LucideIcons.search),
                hintText: 'Search by name, NIP-05, or pubkey',
              ),
              enabled: !isSubmitting.value,
              onChanged: (value) => query.value = value,
            ),
            if (selectedUsers.value.isNotEmpty) ...[
              const SizedBox(height: Grid.xxs),
              Wrap(
                spacing: Grid.half,
                runSpacing: Grid.half,
                children: [
                  for (final user in selectedUsers.value)
                    InputChip(
                      label: Text(user.label),
                      onDeleted: isSubmitting.value
                          ? null
                          : () {
                              selectedUsers.value = [
                                for (final candidate in selectedUsers.value)
                                  if (candidate.pubkey != user.pubkey)
                                    candidate,
                              ];
                            },
                    ),
                ],
              ),
            ],
            const SizedBox(height: Grid.xs),
            SizedBox(
              height: 280,
              child: Builder(
                builder: (context) {
                  if (selectedUsers.value.length >= 8) {
                    return const Center(
                      child: Text(
                        'Direct messages support up to 9 people including you.',
                      ),
                    );
                  }
                  if (debouncedQuery.value.isEmpty) {
                    return const Center(
                      child: Text(
                        'Search for someone to start a conversation.',
                      ),
                    );
                  }
                  if (searchResults.connectionState ==
                      ConnectionState.waiting) {
                    return const Center(child: CircularProgressIndicator());
                  }
                  if (availableResults.isEmpty) {
                    return const Center(child: Text('No matching users.'));
                  }
                  return ListView(
                    shrinkWrap: true,
                    children: [
                      for (final user in availableResults)
                        ListTile(
                          leading: CircleAvatar(
                            backgroundImage: user.avatarUrl != null
                                ? NetworkImage(user.avatarUrl!)
                                : null,
                            child: user.avatarUrl == null
                                ? Text(user.label.substring(0, 1).toUpperCase())
                                : null,
                          ),
                          title: Text(user.label),
                          subtitle: Text(user.secondaryLabel),
                          onTap: () {
                            selectedUsers.value = [
                              ...selectedUsers.value,
                              user,
                            ];
                            queryController.clear();
                            query.value = '';
                            debouncedQuery.value = '';
                          },
                        ),
                    ],
                  );
                },
              ),
            ),
            if (submitError.value case final error?) ...[
              const SizedBox(height: Grid.xxs),
              Text(
                error,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.xs),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                TextButton(
                  onPressed: isSubmitting.value
                      ? null
                      : () => Navigator.of(context).pop(),
                  child: const Text('Cancel'),
                ),
                const SizedBox(width: Grid.half),
                FilledButton(
                  onPressed: canSubmit ? submit : null,
                  child: Text(isSubmitting.value ? 'Opening…' : 'Open DM'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _EphemeralBadge extends StatelessWidget {
  final Channel channel;

  const _EphemeralBadge({required this.channel});

  @override
  Widget build(BuildContext context) {
    final label = _label();

    return Tooltip(
      message: 'Ephemeral channel — cleans up after inactivity',
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
        decoration: BoxDecoration(
          color: const Color(0xFFF59E0B).withValues(alpha: 0.1),
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(
            color: const Color(0xFFF59E0B).withValues(alpha: 0.2),
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(LucideIcons.clock, size: 10, color: _amberColor(context)),
            if (label != null) ...[
              const SizedBox(width: 3),
              Text(
                label,
                style: context.textTheme.labelSmall?.copyWith(
                  fontSize: 10,
                  color: _amberColor(context),
                  fontWeight: FontWeight.w500,
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }

  Color _amberColor(BuildContext context) {
    return context.colors.brightness == Brightness.light
        ? const Color(0xFFB45309)
        : const Color(0xFFFCD34D);
  }

  String? _label() {
    final deadline = channel.ttlDeadline;
    if (deadline == null) return null;
    final diff = deadline.difference(DateTime.now());
    if (diff.isNegative) return 'due';
    if (diff.inMinutes < 60) return '${diff.inMinutes}m';
    if (diff.inHours < 24) return '${diff.inHours}h';
    return '${diff.inDays}d';
  }
}

class _ConnectionBanner extends StatelessWidget {
  final SessionStatus status;

  const _ConnectionBanner({required this.status});

  @override
  Widget build(BuildContext context) {
    if (status == SessionStatus.connected ||
        status == SessionStatus.disconnected) {
      return const SizedBox.shrink();
    }

    final isConnecting = status == SessionStatus.connecting;
    final message = isConnecting ? 'Connecting…' : 'Reconnecting…';

    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.xs,
        vertical: Grid.quarter + 2,
      ),
      color: context.colors.surfaceContainerHighest,
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          SizedBox(
            width: 12,
            height: 12,
            child: CircularProgressIndicator(
              strokeWidth: 2,
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(width: Grid.xxs),
          Text(
            message,
            style: context.textTheme.labelSmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _ErrorView extends StatelessWidget {
  final Object error;
  final VoidCallback onRetry;

  const _ErrorView({required this.error, required this.onRetry});

  static String _userMessage(Object error) {
    if (error is RelayException) {
      if (error.statusCode == 401) {
        return 'Not authorized. Check your API token.';
      }
      if (error.statusCode == 403) {
        return 'Access denied.';
      }
      return 'Server error (${error.statusCode}). Try again later.';
    }
    if (error is SocketException) {
      return 'Could not reach the relay server.';
    }
    return 'Something went wrong. Check your connection.';
  }

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.wifiOff,
              size: Grid.xl,
              color: context.colors.error,
            ),
            const SizedBox(height: Grid.xs),
            Text(
              'Could not load channels',
              style: context.textTheme.titleMedium,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              _userMessage(error),
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              textAlign: TextAlign.center,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: Grid.xs),
            FilledButton.icon(
              onPressed: onRetry,
              icon: const Icon(LucideIcons.refreshCw),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }
}

class _WorkspaceSwitcherSheet extends ConsumerWidget {
  const _WorkspaceSwitcherSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final workspacesAsync = ref.watch(workspaceListProvider);
    final activeAsync = ref.watch(activeWorkspaceProvider);
    final sessionState = ref.watch(relaySessionProvider);

    return SafeArea(
      child: workspacesAsync.when(
        loading: () => const SizedBox(
          height: 120,
          child: Center(child: CircularProgressIndicator()),
        ),
        error: (e, _) => Padding(
          padding: const EdgeInsets.all(Grid.xs),
          child: Text('Error loading workspaces: $e'),
        ),
        data: (workspaces) {
          final activeId = activeAsync.value?.id;
          return Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final workspace in workspaces)
                _WorkspaceSwitcherTile(
                  workspace: workspace,
                  isActive: workspace.id == activeId,
                  sessionStatus: workspace.id == activeId
                      ? sessionState.status
                      : null,
                  onTap: () async {
                    if (workspace.id != activeId) {
                      await ref
                          .read(workspaceListProvider.notifier)
                          .switchWorkspace(workspace.id);
                    }
                    if (context.mounted) Navigator.of(context).pop();
                  },
                  onRename: () async {
                    final nav = Navigator.of(context, rootNavigator: true);
                    final notifier = ref.read(workspaceListProvider.notifier);
                    Navigator.of(context).pop();
                    final name = await showDialog<String>(
                      context: nav.context,
                      useRootNavigator: true,
                      builder: (_) =>
                          _RenameWorkspaceDialog(currentName: workspace.name),
                    );
                    if (name != null && name.isNotEmpty) {
                      await notifier.renameWorkspace(workspace.id, name);
                    }
                  },
                  onRemove: () async {
                    final confirmed = await showDialog<bool>(
                      context: context,
                      builder: (dialogContext) => AlertDialog(
                        title: const Text('Remove Workspace'),
                        content: Text(
                          'Remove "${workspace.name}"? You can re-pair later.',
                        ),
                        actions: [
                          TextButton(
                            onPressed: () =>
                                Navigator.of(dialogContext).pop(false),
                            child: const Text('Cancel'),
                          ),
                          TextButton(
                            onPressed: () =>
                                Navigator.of(dialogContext).pop(true),
                            child: const Text('Remove'),
                          ),
                        ],
                      ),
                    );
                    if (confirmed == true && context.mounted) {
                      final messenger = ScaffoldMessenger.of(context);
                      try {
                        await ref
                            .read(workspaceListProvider.notifier)
                            .removeWorkspace(workspace.id);
                        if (context.mounted) Navigator.of(context).pop();
                      } catch (e) {
                        messenger.showSnackBar(
                          SnackBar(
                            content: Text('Failed to remove workspace: $e'),
                          ),
                        );
                      }
                    }
                  },
                ),
              const Divider(height: 1),
              ListTile(
                leading: const Icon(LucideIcons.plus),
                title: const Text('Add Workspace'),
                onTap: () {
                  final nav = Navigator.of(context, rootNavigator: true);
                  ref.read(pairingProvider.notifier).reset();
                  Navigator.of(context).pop();
                  nav.push(
                    MaterialPageRoute<void>(
                      builder: (_) => const PairingPage(addingWorkspace: true),
                    ),
                  );
                },
              ),
            ],
          );
        },
      ),
    );
  }
}

class _WorkspaceSwitcherTile extends StatelessWidget {
  final Workspace workspace;
  final bool isActive;
  final SessionStatus? sessionStatus;
  final VoidCallback onTap;
  final VoidCallback onRename;
  final VoidCallback onRemove;

  const _WorkspaceSwitcherTile({
    required this.workspace,
    required this.isActive,
    required this.sessionStatus,
    required this.onTap,
    required this.onRename,
    required this.onRemove,
  });

  @override
  Widget build(BuildContext context) {
    final host = Uri.tryParse(workspace.relayUrl)?.host ?? workspace.relayUrl;

    return ListTile(
      leading: _StatusDot(isActive: isActive, sessionStatus: sessionStatus),
      title: Text(
        workspace.name,
        style: context.textTheme.bodyLarge?.copyWith(
          fontWeight: isActive ? FontWeight.w600 : FontWeight.normal,
        ),
      ),
      subtitle: Text(
        host,
        style: context.textTheme.bodySmall?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
      trailing: PopupMenuButton<String>(
        icon: Icon(
          LucideIcons.ellipsisVertical,
          size: 18,
          color: context.colors.onSurfaceVariant,
        ),
        onSelected: (value) {
          switch (value) {
            case 'rename':
              onRename();
            case 'remove':
              onRemove();
          }
        },
        itemBuilder: (_) => [
          const PopupMenuItem(value: 'rename', child: Text('Rename')),
          const PopupMenuItem(value: 'remove', child: Text('Remove')),
        ],
      ),
      onTap: onTap,
    );
  }
}

class _StatusDot extends StatelessWidget {
  final bool isActive;
  final SessionStatus? sessionStatus;

  const _StatusDot({required this.isActive, required this.sessionStatus});

  @override
  Widget build(BuildContext context) {
    if (!isActive) {
      return Container(
        width: 10,
        height: 10,
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          color: context.colors.outline.withValues(alpha: 0.3),
        ),
      );
    }

    final color = switch (sessionStatus) {
      SessionStatus.connected => context.appColors.success,
      SessionStatus.connecting ||
      SessionStatus.reconnecting => context.appColors.warning,
      _ => context.colors.outline,
    };

    return Container(
      width: 10,
      height: 10,
      decoration: BoxDecoration(shape: BoxShape.circle, color: color),
    );
  }
}

class _RenameWorkspaceDialog extends HookWidget {
  final String currentName;

  const _RenameWorkspaceDialog({required this.currentName});

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: currentName);

    return AlertDialog(
      title: const Text('Rename Workspace'),
      content: TextField(
        controller: controller,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Name'),
        onSubmitted: (value) {
          final name = value.trim();
          if (name.isNotEmpty) Navigator.of(context).pop(name);
        },
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(
          onPressed: () {
            final name = controller.text.trim();
            if (name.isNotEmpty) Navigator.of(context).pop(name);
          },
          child: const Text('Rename'),
        ),
      ],
    );
  }
}

class _WorkspaceIndicator extends ConsumerWidget {
  final VoidCallback onTap;

  const _WorkspaceIndicator({required this.onTap});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activeAsync = ref.watch(activeWorkspaceProvider);
    final sessionState = ref.watch(relaySessionProvider);

    final name = activeAsync.value?.name;
    final host = activeAsync.value != null
        ? Uri.tryParse(activeAsync.value!.relayUrl)?.host
        : null;

    return GestureDetector(
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.only(left: Grid.xs),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 8,
              height: 8,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: _statusColor(context, sessionState.status),
              ),
            ),
            const SizedBox(width: Grid.half),
            if (name != null)
              Flexible(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.labelLarge?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    if (host != null)
                      Text(
                        host,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onSurfaceVariant,
                          fontSize: 10,
                        ),
                      ),
                  ],
                ),
              )
            else
              const Text('\u{1F331}', style: TextStyle(fontSize: 28)),
            const SizedBox(width: Grid.quarter),
            Icon(
              LucideIcons.chevronDown,
              size: 14,
              color: context.colors.onSurfaceVariant,
            ),
          ],
        ),
      ),
    );
  }

  Color _statusColor(BuildContext context, SessionStatus status) {
    return switch (status) {
      SessionStatus.connected => context.appColors.success,
      SessionStatus.connecting ||
      SessionStatus.reconnecting => context.appColors.warning,
      _ => context.colors.outline,
    };
  }
}
