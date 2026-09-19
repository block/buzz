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
import '../../shared/widgets/bee_refresh_indicator.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/message_author_meta.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_management_provider.dart';
import '../channels/channels_provider.dart';
import '../channels/dm_channel_labels.dart';
import '../channels/message_content.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import 'activity_provider.dart';
import 'compose_drafts_provider.dart';
import 'dm_resurface.dart';
import 'feed_item.dart';
import 'inbox_item.dart';
import 'inbox_local_state_provider.dart';
import 'inbox_read_state.dart';
import 'reminders_provider.dart';

part 'activity_page/header_actions.dart';
part 'activity_page/inbox_row.dart';
part 'activity_page/lists.dart';
part 'activity_page/status_views.dart';

const double _splitInboxMinListWidth = 300;
const double _splitInboxMaxListWidth = 400;

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
  const ActivityPage({this.tabReselection, this.splitView = false, super.key});

  /// Notifies this page when its already-selected tab is tapped again.
  final ValueListenable<int>? tabReselection;

  /// Keeps the inbox list visible beside its selected conversation.
  final bool splitView;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final feedAsync = ref.watch(activityProvider);
    final channelsAsync = ref.watch(channelsProvider);
    final filter = useState(InboxFilter.all);
    final unreadOnly = useState(false);
    final selectedConversationId = useState<String?>(null);
    final selectedItemTarget = useState<FeedItem?>(null);
    final selectedItemForDetail = useState<InboxItem?>(null);
    final selectedChannelForDetail = useState<Channel?>(null);
    final selectionGeneration = useState(0);
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
    final visibleConversationIdsKey = visibleItems
        .map((item) => item.conversationId)
        .join('\u0000');
    useEffect(() {
      if (!splitView) return null;
      final hasSelectedItem = visibleItems.any(
        (item) => item.conversationId == selectedConversationId.value,
      );
      final retainsSelectedDetail =
          unreadOnly.value &&
          selectedItemForDetail.value?.conversationId ==
              selectedConversationId.value;
      if (!hasSelectedItem && !retainsSelectedDetail) {
        selectedConversationId.value = visibleItems.firstOrNull?.conversationId;
        selectedItemTarget.value = null;
        selectedItemForDetail.value = null;
        selectedChannelForDetail.value = null;
      }
      return null;
    }, [splitView, visibleConversationIdsKey, filter.value, unreadOnly.value]);
    final selectedItem =
        visibleItems.cast<InboxItem?>().firstWhere(
          (item) => item?.conversationId == selectedConversationId.value,
          orElse: () => null,
        ) ??
        (unreadOnly.value ? selectedItemForDetail.value : null);

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

    Future<void> openItem(InboxItem item) async {
      final channelId = item.item.channelId;
      if (channelId == null) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(content: Text("This item isn't linked to a channel.")),
        );
        return;
      }
      var channel = channelById[channelId];
      if (channel == null &&
          myPk != null &&
          ref.read(channelsProvider.notifier).hiddenDmIds.contains(channelId)) {
        final expectedPubkey = myPk.toLowerCase();
        final expectedRelayUrl = ref.read(relayConfigProvider).baseUrl;
        bool isCurrentScope() =>
            context.mounted &&
            ref.read(myPubkeyProvider)?.toLowerCase() == expectedPubkey &&
            ref.read(relayConfigProvider).baseUrl == expectedRelayUrl;
        try {
          final members = await ref.read(
            channelMembersProvider(channelId).future,
          );
          if (!isCurrentScope()) return;
          final peers = dmPeerPubkeysFromMembers(
            members.map((member) => member.pubkey),
            expectedPubkey,
          );
          if (peers.isEmpty) {
            throw StateError('Could not determine the DM membership.');
          }
          final reopened = await ref
              .read(channelActionsProvider)
              .openDm(pubkeys: peers.toList());
          if (!isCurrentScope()) return;
          if (reopened.id != channelId) {
            throw StateError('Relay reopened a different DM conversation.');
          }
          channel = reopened;
        } catch (error) {
          if (!isCurrentScope()) return;
          if (!context.mounted) return;
          ScaffoldMessenger.maybeOf(context)?.showSnackBar(
            SnackBar(content: Text('Could not reopen conversation: $error')),
          );
          return;
        }
      }
      if (channel == null) {
        if (!context.mounted) return;
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(content: Text('Channel not found in this workspace.')),
        );
        return;
      }
      final resolvedChannel = channel;

      // Deep-link to the represented message: oldest unread in the group,
      // falling back to the latest event.
      final readAt = resolveInboxItemReadAt(item, markerOf: markerOf);
      final target = item.deepLinkTarget(readAt);
      final thread = threadReferenceOf(target.tags);
      final threadRootId = isBroadcastReply(target.tags)
          ? null
          : thread.parentId;

      if (splitView) {
        selectedConversationId.value = item.conversationId;
        selectedItemTarget.value = target;
        selectedItemForDetail.value = item;
        selectedChannelForDetail.value = resolvedChannel;
        selectionGeneration.value++;
        markItemRead(item);
        return;
      }

      if (!context.mounted) return;
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(
            channel: resolvedChannel,
            initialMessageId: target.id,
            initialThreadRootId: threadRootId,
            initialThreadRouteBehavior:
                InitialThreadRouteBehavior.replaceCurrentRoute,
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
            initialThreadRouteBehavior:
                InitialThreadRouteBehavior.replaceCurrentRoute,
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
      body = BeeRefreshIndicator(
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
                          selected:
                              splitView &&
                              item.conversationId ==
                                  selectedConversationId.value,
                          onTap: () => unawaited(openItem(item)),
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

    final inboxPane = FrostedScaffold(
      backgroundColor: context.colors.surface,
      appBar: FrostedAppBar(
        automaticallyImplyLeading: false,
        horizontalInset: Grid.gutter,
        showBottomDivider: true,
        bottomDividerOpacity: 0.07,
        title: Text('Activity', style: headerTitleStyle),
        titleStyle: headerTitleStyle,
        actions: [
          _ActivityActionsPill(
            filter: filter.value,
            unreadOnly: unreadOnly.value,
            unreadCount: unreadVisibleCount,
            onFilterChanged: (f) {
              filter.value = f;
              selectedConversationId.value = null;
              selectedItemTarget.value = null;
              selectedItemForDetail.value = null;
              selectedChannelForDetail.value = null;
            },
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

    if (!splitView) return inboxPane;

    final detailTarget =
        selectedItemTarget.value ??
        selectedItem?.deepLinkTarget(
          resolveInboxItemReadAt(selectedItem, markerOf: markerOf),
        );
    final retainedChannel = selectedChannelForDetail.value;
    final detailChannel = detailTarget == null
        ? null
        : retainedChannel?.id == detailTarget.channelId
        ? retainedChannel
        : channelById[detailTarget.channelId];
    final detailThreadRootId =
        detailTarget == null || isBroadcastReply(detailTarget.tags)
        ? null
        : threadReferenceOf(detailTarget.tags).parentId;

    return LayoutBuilder(
      builder: (context, constraints) {
        final listWidth = (constraints.maxWidth / 3)
            .clamp(_splitInboxMinListWidth, _splitInboxMaxListWidth)
            .toDouble();
        return Row(
          children: [
            SizedBox(
              key: const ValueKey('split-activity-inbox-list'),
              width: listWidth,
              child: inboxPane,
            ),
            VerticalDivider(width: 1, color: context.colors.outlineVariant),
            Expanded(
              child: _SplitActivityDetail(
                item: selectedItem,
                channel: detailChannel,
                initialMessageId: detailTarget?.id,
                initialThreadRootId: detailThreadRootId,
                selectionGeneration: selectionGeneration.value,
              ),
            ),
          ],
        );
      },
    );
  }
}

class _SplitActivityDetail extends HookWidget {
  const _SplitActivityDetail({
    required this.item,
    required this.channel,
    required this.initialMessageId,
    required this.initialThreadRootId,
    required this.selectionGeneration,
  });

  final InboxItem? item;
  final Channel? channel;
  final String? initialMessageId;
  final String? initialThreadRootId;
  final int selectionGeneration;

  @override
  Widget build(BuildContext context) {
    if (item == null || channel == null || initialMessageId == null) {
      return const _SplitActivityEmptyDetail();
    }

    final navigatorKey = useMemoized(GlobalKey<NavigatorState>.new, [
      item!.conversationId,
      selectionGeneration,
    ]);
    return NavigatorPopHandler(
      key: ValueKey(
        'split-activity-detail-${item!.conversationId}-$selectionGeneration',
      ),
      onPopWithResult: (_) => navigatorKey.currentState?.maybePop(),
      child: Navigator(
        key: navigatorKey,
        onGenerateRoute: (_) => MaterialPageRoute<void>(
          builder: (_) => ChannelDetailPage(
            channel: channel!,
            initialMessageId: initialMessageId,
            initialThreadRootId: initialThreadRootId,
            initialThreadRouteBehavior:
                InitialThreadRouteBehavior.replaceCurrentRoute,
          ),
        ),
      ),
    );
  }
}

class _SplitActivityEmptyDetail extends StatelessWidget {
  const _SplitActivityEmptyDetail();

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: context.colors.surface,
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.inbox,
              color: context.colors.onSurfaceVariant,
              size: 32,
            ),
            const SizedBox(height: Grid.sm),
            Text(
              'Select an inbox item',
              style: context.textTheme.titleMedium?.copyWith(
                color: context.colors.onSurface,
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              'Its conversation will open here.',
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
