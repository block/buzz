import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' show SemanticsRole;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/message_author_meta.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channels_provider.dart';
import '../channels/dm_channel_labels.dart';
import '../channels/message_content.dart';
import '../channels/read_state/read_state_format.dart';
import '../channels/read_state/read_state_provider.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'activity_provider.dart';
import 'compose_drafts_provider.dart';
import 'inbox_item.dart';
import 'inbox_local_state_provider.dart';
import 'inbox_read_state.dart';
import 'reminders_provider.dart';

part 'activity_page/inbox_row.dart';
part 'activity_page/popover_menu.dart';

/// Conversation-oriented Activity inbox.
///
/// Matches desktop's Home inbox item design and semantics (see
/// `desktop/src/features/home/ui/InboxListPane.tsx`): full sender avatar +
/// name, contextual "Mentioned in #channel"-style label, unread dot + time,
/// message preview — while keeping mobile's list → canonical destination
/// navigation. Row taps deep-link to the represented message (oldest unread
/// for grouped conversations) rather than just opening the channel.
class ActivityPage extends HookConsumerWidget {
  const ActivityPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final feedAsync = ref.watch(activityProvider);
    final channelsAsync = ref.watch(channelsProvider);
    final filter = useState(InboxFilter.all);
    final unreadOnly = useState(false);
    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
    );

    final readState = ref.watch(readStateProvider);
    final localState = ref.watch(inboxLocalStateProvider);
    final drafts = ref.watch(composeDraftsProvider);
    final dueReminderCount = ref.watch(dueReminderCountProvider);
    final allItems = ref.watch(inboxItemsProvider);
    final myPk = ref.watch(myPubkeyProvider);

    // Cache the last non-empty feed so the UI doesn't flash on rebuild.
    final hasLoadedOnce = useRef(false);
    if (feedAsync.hasValue) hasLoadedOnce.value = true;

    final channels = channelsAsync.asData?.value ?? const <Channel>[];
    final channelById = {for (final c in channels) c.id: c};

    int? markerOf(String contextId) => readState.effectiveTimestamp(contextId);
    bool isDone(InboxItem item) => isInboxItemDone(
      item,
      markerOf: markerOf,
      localUnreadOverrides: localState.unreadIds,
      localDoneSet: localState.doneIds,
    );

    final visibleItems = [
      for (final item in allItems)
        if (matchesInboxFilter(item, filter.value) &&
            (!unreadOnly.value || !isDone(item)))
          item,
    ];

    // Preload sender profiles for visible rows.
    final pubkeys = visibleItems.map((i) => i.item.pubkey).toSet().toList();
    ref.read(userCacheProvider.notifier).preload(pubkeys);

    final unreadVisibleCount = visibleItems.where((i) => !isDone(i)).length;

    void markItemRead(InboxItem item) {
      final notifier = ref.read(readStateProvider.notifier);
      ref
          .read(inboxLocalStateProvider.notifier)
          .clearUnread(groupedInboxItemIds(item));
      final threadRootId = item.threadRootId;
      if (threadRootId != null) {
        notifier.markContextRead(
          threadContextKey(threadRootId),
          item.latestActivityAt,
        );
        final channelRead = groupedChannelReadTimestamp(item);
        if (channelRead != null) {
          notifier.markContextRead(
            channelRead.channelId,
            channelRead.timestamp,
          );
        }
        return;
      }
      final channelId = item.item.channelId;
      if (channelId != null) {
        notifier.markContextRead(channelId, item.latestActivityAt);
        ref
            .read(channelsProvider.notifier)
            .clearObservedUnreadCoveredByRead(channelId, item.latestActivityAt);
        return;
      }
      ref.read(inboxLocalStateProvider.notifier).markDone(item.id);
    }

    void markItemUnread(InboxItem item) {
      ref
          .read(inboxLocalStateProvider.notifier)
          .markUnread(groupedInboxItemIds(item));
    }

    void openItem(InboxItem item) {
      final channelId = item.item.channelId;
      if (channelId == null) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(content: Text("This item isn't linked to a channel.")),
        );
        return;
      }
      final channel = channelById[channelId];
      if (channel == null) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(content: Text('Channel not found in this workspace.')),
        );
        return;
      }

      // Deep-link to the represented message: oldest unread in the group,
      // falling back to the latest event.
      final readAt = resolveInboxItemReadAt(item, markerOf: markerOf);
      final target = item.deepLinkTarget(readAt);
      final thread = threadReferenceOf(target.tags);
      final threadRootId = isBroadcastReply(target.tags)
          ? null
          : (thread.rootId ?? thread.parentId);

      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(
            channel: channel,
            initialMessageId: target.id,
            initialThreadRootId: threadRootId,
          ),
        ),
      );
    }

    void openDraft(ComposeDraft draft) {
      final channel = channelById[draft.channelId];
      if (channel == null) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(
            content: Text('Channel for this draft is no longer available.'),
          ),
        );
        return;
      }
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(
            channel: channel,
            initialThreadRootId: draft.threadHeadId,
          ),
        ),
      );
    }

    void openReminder(Reminder reminder) {
      final target = reminder.target;
      if (target == null) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(
            content: Text("This reminder isn't linked to a message."),
          ),
        );
        return;
      }
      final channel = channelById[target.channelId];
      if (channel == null) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(
            content: Text('Channel for this reminder is no longer available.'),
          ),
        );
        return;
      }
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(
            channel: channel,
            initialMessageId: target.eventId,
          ),
        ),
      );
    }

    Future<void> refresh() async {
      await Future.wait([
        ref.read(activityProvider.notifier).refresh(),
        ref.read(remindersProvider.notifier).refresh(),
      ]);
    }

    final Widget body;
    if (filter.value == InboxFilter.reminders) {
      body = _RemindersList(onOpen: openReminder, onRefresh: refresh);
    } else if (filter.value == InboxFilter.drafts) {
      body = _DraftsList(
        drafts: drafts,
        channelById: channelById,
        myPubkey: myPk,
        onOpen: openDraft,
        onDelete: (draft) =>
            ref.read(composeDraftsProvider.notifier).remove(draft.key),
      );
    } else if (feedAsync.hasError && allItems.isEmpty) {
      body = _ErrorView(onRetry: refresh);
    } else if (!hasLoadedOnce.value && allItems.isEmpty) {
      body = const _LoadingSkeleton();
    } else if (visibleItems.isEmpty) {
      body = _EmptyFilterState(
        filter: filter.value,
        unreadOnly: unreadOnly.value,
      );
    } else {
      // Compute the "New" boundary: index of the first unread row when the
      // rows above it are read (list is newest-first, so unread rows sit on
      // top; the divider marks where the unread block ends).
      final firstReadIndex = visibleItems.indexWhere(isDone);
      final newBoundaryIndex = !unreadOnly.value && firstReadIndex > 0
          ? firstReadIndex
          : -1;

      body = RefreshIndicator(
        onRefresh: refresh,
        child: ListView.builder(
          padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
          itemCount: visibleItems.length,
          itemBuilder: (context, index) {
            final item = visibleItems[index];
            final channel = item.item.channelId != null
                ? channelById[item.item.channelId]
                : null;
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (index == newBoundaryIndex) const _NewBoundaryDivider(),
                _InboxRow(
                  item: item,
                  channel: channel,
                  currentPubkey: myPk,
                  isDone: isDone(item),
                  onTap: () => openItem(item),
                  onMarkRead: () => markItemRead(item),
                  onMarkUnread: () => markItemUnread(item),
                ),
              ],
            );
          },
        ),
      );
    }

    return FrostedScaffold(
      appBar: FrostedAppBar(
        gradient: context.appColors.topSectionGradient,
        automaticallyImplyLeading: false,
        title: const Text('Activity'),
        titleStyle: headerTitleStyle,
        actions: [
          _FilterMenuButton(
            filter: filter.value,
            dueReminderCount: dueReminderCount,
            draftCount: drafts.length,
            onChanged: (f) => filter.value = f,
          ),
          _InboxOptionsButton(
            unreadOnly: unreadOnly.value,
            unreadCount: unreadVisibleCount,
            onUnreadOnlyChanged: (v) => unreadOnly.value = v,
            onMarkAllRead: () {
              for (final item in visibleItems) {
                if (!isDone(item)) markItemRead(item);
              }
            },
          ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: Padding(
          padding: EdgeInsets.only(
            top: frostedAppBarHeight(context, titleStyle: headerTitleStyle),
          ),
          child: body,
        ),
      ),
    );
  }
}

const _filterLabels = <InboxFilter, String>{
  InboxFilter.all: 'All',
  InboxFilter.mention: 'Mentions',
  InboxFilter.thread: 'Threads',
  InboxFilter.needsAction: 'Needs Action',
  InboxFilter.activity: 'Activity',
  InboxFilter.agentActivity: 'Agents',
  InboxFilter.reminders: 'Reminders',
  InboxFilter.drafts: 'Drafts',
};

/// Compact filter dropdown replacing the old chip rail — mirrors desktop's
/// inbox filter menu (`FILTER_OPTIONS`).
class _FilterMenuButton extends StatelessWidget {
  final InboxFilter filter;
  final int dueReminderCount;
  final int draftCount;
  final ValueChanged<InboxFilter> onChanged;

  const _FilterMenuButton({
    required this.filter,
    required this.dueReminderCount,
    required this.draftCount,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    return Builder(
      builder: (buttonContext) => InkWell(
        key: const ValueKey('activity-filter-menu'),
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: () async {
          final selected = await _showActivityPopover<InboxFilter>(
            context: buttonContext,
            width: 240,
            alignment: _ActivityPopoverAlignment.start,
            offset: const Offset(0, Grid.half),
            menuPadding: const EdgeInsets.symmetric(vertical: Grid.half),
            color: context.colors.surface.withValues(alpha: 0.98),
            elevation: 8,
            shadowColor: context.colors.shadow.withValues(alpha: 0.18),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(Radii.card),
              side: BorderSide(
                color: context.colors.outlineVariant.withValues(alpha: 0.45),
              ),
            ),
            surfaceKey: const ValueKey('activity-filter-popover'),
            items: [
              for (final entry in _filterLabels.entries)
                PopupMenuItem(
                  value: entry.key,
                  height: Grid.xl,
                  padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
                  child: Row(
                    children: [
                      SizedBox(
                        width: Grid.sm,
                        child: entry.key == filter
                            ? Icon(
                                LucideIcons.check,
                                size: 16,
                                color: context.colors.primary,
                              )
                            : null,
                      ),
                      Expanded(
                        child: Text(
                          entry.value,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: context.textTheme.labelLarge?.copyWith(
                            color: context.colors.onSurface,
                          ),
                        ),
                      ),
                      if (entry.key == InboxFilter.reminders &&
                          dueReminderCount > 0)
                        _CountBadge(count: dueReminderCount)
                      else if (entry.key == InboxFilter.drafts &&
                          draftCount > 0)
                        _CountBadge(count: draftCount),
                    ],
                  ),
                ),
            ],
          );
          if (buttonContext.mounted && selected != null) onChanged(selected);
        },
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                _filterLabels[filter]!,
                style: context.textTheme.labelLarge?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
              const SizedBox(width: Grid.quarter),
              Icon(
                LucideIcons.chevronDown,
                size: 16,
                color: context.colors.onSurfaceVariant,
              ),
              if (dueReminderCount > 0 || draftCount > 0) ...[
                const SizedBox(width: Grid.quarter),
                Container(
                  width: 6,
                  height: 6,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: context.colors.primary,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _CountBadge extends StatelessWidget {
  final int count;

  const _CountBadge({required this.count});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.half + Grid.quarter,
        vertical: Grid.quarter,
      ),
      decoration: BoxDecoration(
        color: context.colors.primary,
        borderRadius: BorderRadius.circular(Grid.xxs),
      ),
      child: Text(
        '$count',
        style: context.textTheme.labelSmall?.copyWith(
          color: context.colors.onPrimary,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}

/// Overflow menu with the unread-only toggle and mark-all-read, mirroring
/// desktop's inbox options popover.
class _InboxOptionsButton extends StatelessWidget {
  final bool unreadOnly;
  final int unreadCount;
  final ValueChanged<bool> onUnreadOnlyChanged;
  final VoidCallback onMarkAllRead;

  const _InboxOptionsButton({
    required this.unreadOnly,
    required this.unreadCount,
    required this.onUnreadOnlyChanged,
    required this.onMarkAllRead,
  });

  @override
  Widget build(BuildContext context) {
    return Builder(
      builder: (buttonContext) => IconButton(
        key: const ValueKey('activity-options-menu'),
        tooltip: 'Activity options',
        icon: const Icon(LucideIcons.ellipsis, size: 20),
        onPressed: () async {
          final selected = await _showActivityPopover<String>(
            context: buttonContext,
            width: 216,
            alignment: _ActivityPopoverAlignment.end,
            color: context.colors.surface,
            elevation: 4,
            shadowColor: context.colors.shadow.withValues(alpha: 0.18),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(Radii.md),
              side: BorderSide(color: context.colors.outline),
            ),
            surfaceKey: const ValueKey('activity-options-popover'),
            items: [
              PopupMenuItem(
                value: 'unread-only',
                child: Row(
                  children: [
                    Expanded(
                      child: Text(unreadOnly ? 'Show all' : 'Show unread'),
                    ),
                    if (unreadOnly)
                      Icon(
                        LucideIcons.check,
                        size: 16,
                        color: context.colors.primary,
                      ),
                  ],
                ),
              ),
              PopupMenuItem(
                value: 'mark-all-read',
                enabled: unreadCount > 0,
                child: Row(
                  children: [
                    const Expanded(child: Text('Mark all as read')),
                    if (unreadCount > 0)
                      Text(
                        '$unreadCount',
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                  ],
                ),
              ),
            ],
          );
          if (!buttonContext.mounted || selected == null) return;
          if (selected == 'unread-only') onUnreadOnlyChanged(!unreadOnly);
          if (selected == 'mark-all-read') onMarkAllRead();
        },
      ),
    );
  }
}

/// "New" boundary between unread and previously read rows.
class _NewBoundaryDivider extends StatelessWidget {
  const _NewBoundaryDivider();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.half,
      ),
      child: Row(
        children: [
          Expanded(
            child: Divider(color: context.colors.error.withValues(alpha: 0.5)),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
            child: Text(
              'New',
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.error,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          Expanded(
            child: Divider(color: context.colors.error.withValues(alpha: 0.5)),
          ),
        ],
      ),
    );
  }
}

/// Reminders surface for the Reminders filter — due/pending NIP-ER
/// reminders that deep-link to their target message.
class _RemindersList extends ConsumerWidget {
  final void Function(Reminder reminder) onOpen;
  final Future<void> Function() onRefresh;

  const _RemindersList({required this.onOpen, required this.onRefresh});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final remindersAsync = ref.watch(remindersProvider);
    final reminders = [
      for (final r in remindersAsync.value ?? const <Reminder>[])
        if (r.status == 'pending') r,
    ];

    if (remindersAsync.isLoading && reminders.isEmpty) {
      return const Center(child: CircularProgressIndicator());
    }
    if (reminders.isEmpty) {
      return const _EmptySurface(
        icon: LucideIcons.clock,
        message: 'No reminders',
        detail: 'Reminders you set will show up here.',
      );
    }

    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    return RefreshIndicator(
      onRefresh: onRefresh,
      child: ListView.builder(
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        itemCount: reminders.length,
        itemBuilder: (context, index) {
          final reminder = reminders[index];
          final due = reminder.isDue(now);
          final title = reminder.note?.trim().isNotEmpty == true
              ? reminder.note!.trim()
              : reminder.target?.preview ?? 'Reminder';
          return ListTile(
            key: ValueKey('reminder-row-${reminder.id}'),
            leading: Icon(
              due ? LucideIcons.bellRing : LucideIcons.clock,
              size: 20,
              color: due
                  ? context.colors.primary
                  : context.colors.onSurfaceVariant,
            ),
            title: Text(title, maxLines: 2, overflow: TextOverflow.ellipsis),
            subtitle: reminder.notBefore != null
                ? Text(
                    due
                        ? 'Due now'
                        : 'Due ${_inboxTimestamp(reminder.notBefore!)}',
                  )
                : null,
            onTap: () => onOpen(reminder),
          );
        },
      ),
    );
  }
}

/// Drafts surface for the Drafts filter — locally saved unsent composer
/// text that reopens the target composer.
class _DraftsList extends StatelessWidget {
  final List<ComposeDraft> drafts;
  final Map<String, Channel> channelById;
  final String? myPubkey;
  final void Function(ComposeDraft draft) onOpen;
  final void Function(ComposeDraft draft) onDelete;

  const _DraftsList({
    required this.drafts,
    required this.channelById,
    required this.myPubkey,
    required this.onOpen,
    required this.onDelete,
  });

  @override
  Widget build(BuildContext context) {
    if (drafts.isEmpty) {
      return const _EmptySurface(
        icon: LucideIcons.filePen,
        message: 'No drafts',
        detail: 'Unsent messages you start composing will show up here.',
      );
    }

    return ListView.builder(
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
      itemCount: drafts.length,
      itemBuilder: (context, index) {
        final draft = drafts[index];
        final channel = channelById[draft.channelId];
        final destination = channel == null
            ? 'Unavailable channel'
            : channel.isDm
            ? resolveDmChannelDisplayLabel(channel, currentPubkey: myPubkey)
            : '#${channel.name}';
        return ListTile(
          key: ValueKey('draft-row-${draft.key}'),
          leading: Icon(
            LucideIcons.filePen,
            size: 20,
            color: context.colors.onSurfaceVariant,
          ),
          title: Text(draft.text, maxLines: 2, overflow: TextOverflow.ellipsis),
          subtitle: Text(
            draft.threadHeadId != null ? 'Thread in $destination' : destination,
          ),
          trailing: IconButton(
            icon: const Icon(LucideIcons.trash2, size: 18),
            tooltip: 'Delete draft',
            onPressed: () => onDelete(draft),
          ),
          onTap: () => onOpen(draft),
        );
      },
    );
  }
}

/// Inbox list timestamp — desktop's `formatInboxTimestamp`:
/// time today, "Yesterday", short date this year, dated otherwise.
String _inboxTimestamp(int unixSeconds, {DateTime? now}) {
  final current = now ?? DateTime.now();
  final date = DateTime.fromMillisecondsSinceEpoch(unixSeconds * 1000);
  final today = DateTime(current.year, current.month, current.day);
  final day = DateTime(date.year, date.month, date.day);
  final dayDiff = today.difference(day).inDays;

  if (dayDiff == 0) {
    final hour = date.hour % 12 == 0 ? 12 : date.hour % 12;
    final minute = date.minute.toString().padLeft(2, '0');
    return '$hour:$minute ${date.hour < 12 ? 'AM' : 'PM'}';
  }
  if (dayDiff == 1) return 'Yesterday';

  const months = [
    'Jan',
    'Feb',
    'Mar',
    'Apr',
    'May',
    'Jun',
    'Jul',
    'Aug',
    'Sep',
    'Oct',
    'Nov',
    'Dec',
  ];
  final label = '${months[date.month - 1]} ${date.day}';
  return date.year == current.year ? label : '$label, ${date.year}';
}

class _LoadingSkeleton extends StatelessWidget {
  const _LoadingSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView.separated(
      padding: const EdgeInsets.all(Grid.gutter),
      itemCount: 8,
      separatorBuilder: (_, _) => const SizedBox(height: Grid.xs),
      itemBuilder: (context, _) => Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 36,
            height: 36,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: context.colors.outlineVariant.withValues(alpha: 0.4),
            ),
          ),
          const SizedBox(width: Grid.twelve),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Container(
                  width: 140,
                  height: 12,
                  decoration: BoxDecoration(
                    color: context.colors.outlineVariant.withValues(alpha: 0.4),
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
                const SizedBox(height: Grid.xxs),
                Container(
                  width: double.infinity,
                  height: 10,
                  decoration: BoxDecoration(
                    color: context.colors.outlineVariant.withValues(alpha: 0.3),
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
                const SizedBox(height: Grid.half),
                Container(
                  width: 220,
                  height: 10,
                  decoration: BoxDecoration(
                    color: context.colors.outlineVariant.withValues(alpha: 0.2),
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _EmptySurface extends StatelessWidget {
  final IconData icon;
  final String message;
  final String? detail;

  const _EmptySurface({required this.icon, required this.message, this.detail});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: Grid.lg, color: context.colors.onSurfaceVariant),
            const SizedBox(height: Grid.xxs),
            Text(
              message,
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            if (detail != null) ...[
              const SizedBox(height: Grid.half),
              Text(
                detail!,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
                textAlign: TextAlign.center,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _EmptyFilterState extends StatelessWidget {
  final InboxFilter filter;
  final bool unreadOnly;

  const _EmptyFilterState({required this.filter, required this.unreadOnly});

  @override
  Widget build(BuildContext context) {
    if (unreadOnly) {
      return const _EmptySurface(
        icon: LucideIcons.mailOpen,
        message: 'No unread messages',
        detail: 'Turn off the unread filter to see read messages.',
      );
    }
    final (icon, message) = switch (filter) {
      InboxFilter.mention => (LucideIcons.atSign, 'No mentions yet'),
      InboxFilter.thread => (
        LucideIcons.messageSquare,
        'No thread replies yet',
      ),
      InboxFilter.needsAction => (
        LucideIcons.circleAlert,
        'Nothing needs your action',
      ),
      InboxFilter.activity => (
        LucideIcons.activity,
        'No recent channel activity',
      ),
      InboxFilter.agentActivity => (LucideIcons.bot, 'No agent updates'),
      _ => (LucideIcons.bell, 'No activity yet'),
    };
    return _EmptySurface(icon: icon, message: message);
  }
}

class _ErrorView extends StatelessWidget {
  final Future<void> Function() onRetry;

  const _ErrorView({required this.onRetry});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            LucideIcons.triangleAlert,
            size: Grid.lg,
            color: context.colors.error,
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            'Failed to load activity',
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.xs),
          FilledButton.icon(
            onPressed: () => unawaited(onRetry()),
            icon: const Icon(LucideIcons.refreshCcw, size: 16),
            label: const Text('Retry'),
          ),
        ],
      ),
    );
  }
}
