import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/mentions/mention_tags.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/anchored_popover_menu.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/message_author_meta.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channels_provider.dart';
import '../channels/dm_channel_labels.dart';
import '../channels/message_content.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'activity_provider.dart';
import 'compose_drafts_provider.dart';
import 'inbox_item.dart';
import 'inbox_local_state_provider.dart';
import 'inbox_read_state.dart';
import 'reminders_provider.dart';

part 'activity_page/header_actions.dart';
part 'activity_page/inbox_row.dart';
part 'activity_page/lists.dart';
part 'activity_page/status_views.dart';

EdgeInsets _activityScrollPadding(
  BuildContext context, {
  double horizontal = 0,
  double top = Grid.xxs,
  double bottom = Grid.xxs,
}) => EdgeInsets.fromLTRB(
  horizontal,
  top,
  horizontal,
  MediaQuery.paddingOf(context).bottom + bottom,
);

/// Conversation-oriented Activity inbox.
///
/// Matches desktop's Home inbox item design and semantics (see
/// `desktop/src/features/home/ui/InboxListPane.tsx`): full sender avatar +
/// name, contextual "Mentioned in #channel"-style label, unread dot + time,
/// message preview — while keeping mobile's list → canonical destination
/// navigation. Row taps deep-link to the represented message (oldest unread
/// for grouped conversations) rather than just opening the channel.
class ActivityPage extends HookConsumerWidget {
  const ActivityPage({this.tabReselection, super.key});

  /// Notifies this page when its already-selected tab is tapped again.
  final ValueListenable<int>? tabReselection;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final feedAsync = ref.watch(activityProvider);
    final channelsAsync = ref.watch(channelsProvider);
    final filter = useState(InboxFilter.all);
    final unreadOnly = useState(false);
    final scrollController = useScrollController();
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    useEffect(() {
      final tabReselection = this.tabReselection;
      if (tabReselection == null) return null;

      void scrollToTop() {
        if (!scrollController.hasClients) return;
        final position = scrollController.position;
        if (position.pixels <= position.minScrollExtent + 0.5) return;
        if (reducedMotion) {
          scrollController.jumpTo(position.minScrollExtent);
          return;
        }
        unawaited(
          scrollController.animateTo(
            position.minScrollExtent,
            duration: const Duration(milliseconds: 260),
            curve: Curves.easeOutCubic,
          ),
        );
      }

      tabReselection.addListener(scrollToTop);
      return () => tabReselection.removeListener(scrollToTop);
    }, [tabReselection, scrollController, reducedMotion]);
    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
      color: navigationPrimaryForeground(context),
    );
    final topSectionHeight = frostedAppBarHeight(
      context,
      titleStyle: headerTitleStyle,
      bottomHeight: Grid.xxs,
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
    final preloadPubkeys = {
      for (final item in visibleItems) item.item.pubkey.toLowerCase(),
      for (final item in visibleItems)
        ...mentionedPubkeysFromTags(item.item.tags),
    }.toList()..sort();
    final preloadPubkeysKey = preloadPubkeys.join('\u0000');
    useEffect(() {
      ref.read(userCacheProvider.notifier).preload(preloadPubkeys);
      return null;
    }, [preloadPubkeysKey]);

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
          : thread.parentId;

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

    late final Widget body;
    var bodyRidesOverTopSection = false;
    if (filter.value == InboxFilter.reminders) {
      body = _RemindersList(
        scrollController: scrollController,
        onOpen: openReminder,
        onRefresh: refresh,
      );
    } else if (filter.value == InboxFilter.drafts) {
      body = _DraftsList(
        drafts: drafts,
        scrollController: scrollController,
        channelById: channelById,
        myPubkey: myPk,
        onOpen: openDraft,
        onDelete: (draft) =>
            ref.read(composeDraftsProvider.notifier).remove(draft.key),
      );
    } else if (feedAsync.hasError && allItems.isEmpty) {
      body = _ErrorView(onRetry: refresh);
    } else if (!hasLoadedOnce.value && allItems.isEmpty) {
      body = _LoadingSkeleton(scrollController: scrollController);
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

      bodyRidesOverTopSection = true;
      body = RefreshIndicator(
        edgeOffset: topSectionHeight,
        onRefresh: refresh,
        child: CustomScrollView(
          controller: scrollController,
          slivers: [
            SliverToBoxAdapter(child: SizedBox(height: topSectionHeight)),
            DecoratedSliver(
              decoration: BoxDecoration(
                color: context.colors.surface,
                borderRadius: const BorderRadius.vertical(
                  top: Radius.circular(Radii.dialog),
                ),
              ),
              sliver: SliverPadding(
                padding: _activityScrollPadding(context),
                sliver: SliverList.builder(
                  itemCount: visibleItems.length,
                  itemBuilder: (context, index) {
                    final item = visibleItems[index];
                    final channel = item.item.channelId != null
                        ? channelById[item.item.channelId]
                        : null;
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        if (index == newBoundaryIndex)
                          const _NewBoundaryDivider(),
                        _InboxRow(
                          key: ValueKey(item.id),
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
              ),
            ),
          ],
        ),
      );
    }

    return FrostedScaffold(
      backgroundColor: context.colors.surface,
      appBar: FrostedAppBar(
        automaticallyImplyLeading: false,
        horizontalInset: Grid.gutter,
        showBottomDivider: true,
        bottomDividerOpacity: 0.06,
        title: Text('Activity', style: headerTitleStyle),
        titleStyle: headerTitleStyle,
        actions: [
          _ActivityActionsPill(
            filter: filter.value,
            dueReminderCount: dueReminderCount,
            draftCount: drafts.length,
            unreadOnly: unreadOnly.value,
            unreadCount: unreadVisibleCount,
            onFilterChanged: (f) => filter.value = f,
            onUnreadOnlyChanged: (v) => unreadOnly.value = v,
            onMarkAllRead: () {
              for (final item in visibleItems) {
                if (!isDone(item)) markItemRead(item);
              }
            },
          ),
        ],
        bottomHeight: Grid.xxs,
        bottom: const SizedBox.expand(),
      ),
      body: SafeArea(
        key: const ValueKey('activity-content-safe-area'),
        top: false,
        bottom: false,
        child: bodyRidesOverTopSection
            ? body
            : Padding(
                padding: EdgeInsets.only(top: topSectionHeight),
                child: body,
              ),
      ),
    );
  }
}
