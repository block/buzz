import '../channel_sections/section_show_more.dart';
import '../channel_stars/channel_stars_provider.dart';
import '../../../shared/widgets/native_conversation_menu.dart';
import 'dart:async';
import 'dart:convert';
import '../channel_filter/channel_filter_activity.dart';
import '../channel_sort/channel_sort_provider.dart';
import '../channel_sort/channel_sort_storage.dart';
import '../channel_filter/channel_filter_provider.dart';
import '../channel_filter/channel_filter_store.dart';
import '../channels_page/filters.dart';

import 'package:buzz/shared/widgets/feedback_snackbar.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import '../../../shared/profile/user_cache_provider.dart';
import '../../../shared/widgets/concentric_sheet_surface.dart';
import '../../../shared/widgets/bee_refresh_indicator.dart';
import 'me_section.dart';
import 'me_working_provider.dart';
import 'solo_working_dots.dart';
import 'me_unread_provider.dart';
import '../../../shared/widgets/avatar_image.dart';
import '../../../shared/me/me_groups.dart';
import '../../../shared/me/default_agent.dart';
import '../../../shared/me/me_chat_title.dart';
import '../../../shared/me/solo_chat_seed.dart';
import '../../../shared/drafts/compose_drafts_provider.dart';
import '../../../shared/crypto/event_signing.dart';
import '../../../shared/me/me_chat_creation.dart';
import '../../../shared/me/me_sync.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/me/me_recovery.dart';
import '../../../shared/me/me_assistant_names.dart';
import '../../../shared/me/me_assistants.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/frosted_app_bar.dart';
import '../../../shared/widgets/frosted_scaffold.dart';
import '../../../shared/widgets/buzz_loading_indicator.dart';
import '../../../shared/widgets/modal_presentation.dart';
import '../channel.dart';
import '../compose_bar.dart';
import 'me_assistant_picker_sheet.dart';
import 'new_me_project.dart';
import 'me_workspace_picker.dart';
import 'me_recovery_dialog.dart';
import '../channel_actions_sheet.dart';
import '../../../shared/share/share_suggestions.dart';
import '../channel_detail_page.dart';
import '../channel_management_provider.dart';
import '../channels_provider.dart';
import '../channel_sections/channel_sections_provider.dart';

part 'me_page_helpers.dart';
part 'me_empty_state.dart';
part 'new_me_chat.dart';
part 'me_chat_draft.dart';

/// Personal chats and projects shared with the desktop Me workspace.
class MePage extends HookConsumerWidget {
  const MePage({
    this.newChatAction,
    this.newWorkspaceAction,
    this.sliver = false,
    super.key,
  });

  final bool sliver;
  final ValueNotifier<VoidCallback?>? newWorkspaceAction;

  /// Publishes the current chat action to the persistent Home footer.
  final ValueNotifier<VoidCallback?>? newChatAction;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
      color: navigationPrimaryForeground(context),
    );
    final topSectionHeight = sliver
        ? 0.0
        : frostedAppBarHeight(
            context,
            nativeLargeTitle: true,
            titleStyle: headerTitleStyle,
            bottomHeight: Grid.xxs,
          );
    // Warm the directory while browsing Me, before the creation form opens.
    final assistants = ref.watch(meAssistantsProvider).value;
    ref.watch(defaultAgentProvider);
    useEffect(() {
      if (assistants != null) {
        ref.read(userCacheProvider.notifier).preload([
          for (final agent in assistants) agent.pubkey,
        ]);
      }
      return null;
    }, [assistants]);
    final groups = ref.watch(meGroupsProvider);
    final offlineSaveFailed =
        ref.watch(meOfflineSaveWarningProvider).value ?? false;
    final channels = ref.watch(channelsProvider);
    // Rows read unread state themselves; keep it alive while Solo is shown.
    final unreadIds = ref.watch(meUnreadChannelIdsProvider);
    final busy = useState(false);
    Future<void> perform(Future<void> Function() action) async {
      if (busy.value) return;
      busy.value = true;
      try {
        await action();
      } catch (error) {
        if (error is MeRecoveryConflict && context.mounted) {
          await offerMeRecovery(context, ref.read(meSyncProvider));
        }
        if (context.mounted) {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            SnackBar(
              content: Text(error.toString().replaceFirst('Bad state: ', '')),
            ),
            kind: ToastKind.error,
          );
        }
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    // Desktop's unified workspaces are read-only on mobile.
    final unified = ref.watch(
      channelSectionsProvider.select((state) => !state.editable),
    );
    final VoidCallback? newProject = busy.value || !groups.hasValue || unified
        ? null
        : () => perform(() async {
            // The footer can hold this callback from before the unified
            // record arrived.
            if (!ref.read(channelSectionsProvider).editable) return;
            await createMeProject(context, ref);
          });
    final VoidCallback? newChat = busy.value || !groups.hasValue
        ? null
        : () => perform(() => _openNewMeChat(context, ref));

    useEffect(
      () {
        var active = true;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (active) {
            newChatAction?.value = newChat;
            newWorkspaceAction?.value = newProject;
          }
        });
        return () => active = false;
      },
      [newChatAction, newWorkspaceAction, busy.value, groups.hasValue, unified],
    );

    Widget inventoryStatus(Widget child) => sliver
        ? SliverToBoxAdapter(
            child: Padding(
              padding: const EdgeInsets.all(Grid.gutter),
              child: child,
            ),
          )
        : child;
    // Labelled chats come from the channel list, so they stay reachable while
    // the optional filing record loads or fails. Until it has a value, show
    // its status above an unfiled list and never treat it as empty.
    final legacyChannels = groups.value?.channels ?? const <String>[];
    final organizationStatus = groups.when<Widget?>(
      loading: () => const Center(child: BuzzLoadingIndicator()),
      error: (_, _) =>
          _MeRetry(onRetry: () => ref.invalidate(meGroupsProvider)),
      data: (_) => null,
    );
    final organization = organizationStatus == null ? groups.value : null;
    final sections = organization?.groups ?? const [];
    final assignments = organization?.assignments ?? const {};
    final offlineNotice = offlineSaveFailed
        ? SliverToBoxAdapter(
            child: Semantics(
              liveRegion: true,
              child: Column(
                children: [
                  const Text(
                    'Your chats are available, but couldn’t be saved for offline use.',
                  ),
                  TextButton(
                    onPressed: () => ref.invalidate(meGroupsProvider),
                    child: const Text('Retry offline save'),
                  ),
                ],
              ),
            ),
          )
        : null;
    final inventory = channels.when(
      loading: () =>
          inventoryStatus(const Center(child: BuzzLoadingIndicator())),
      error: (_, _) => inventoryStatus(
        _MeRetry(onRetry: () => ref.invalidate(channelsProvider)),
      ),
      data: (all) {
        final visible =
            all
                .where(
                  (c) =>
                      c.isMember &&
                      !c.isArchived &&
                      c.isSoloChat(legacyChannels),
                )
                .toList()
              ..sort(
                (a, b) => (b.lastMessageAt ?? b.createdAt).compareTo(
                  a.lastMessageAt ?? a.createdAt,
                ),
              );
        Widget row(Channel channel, [VoidCallback? onTap]) => SoloChatTile(
          key: ValueKey(channel.id),
          channel: channel,
          onTap: onTap,
        );

        if (sliver) {
          // Home files Solo chats with the rest of the sidebar, as desktop
          // does. This slot holds the chats left in Solo itself.
          final layout = ref.watch(sidebarLayoutProvider);
          final stars = ref.watch(channelStarsProvider).store.channels;
          final unfiled = sortChannelsForList([
            for (final channel in visible)
              if (layout.sectionKeyFor(
                    channel.id,
                    starred: stars[channel.id]?.enabled == true,
                    home: 'solo',
                  ) ==
                  'solo')
                channel,
          ], ref.watch(channelSortProvider).sortModeFor('solo'));
          return SliverMainAxisGroup(
            slivers: [
              if (organizationStatus != null)
                SliverToBoxAdapter(
                  child: Padding(
                    padding: const EdgeInsets.all(Grid.gutter),
                    child: organizationStatus,
                  ),
                ),
              ?offlineNotice,
              _SoloFilterSection(
                key: const ValueKey('me-ungrouped-chats'),
                group: 'solo',
                label: 'Solo',
                chats: unfiled,
                unread: unreadIds,
                row: row,
                empty: organization == null ? null : 'No chats yet',
              ),
            ],
          );
        }

        Widget section(
          String group,
          String label,
          Iterable<Channel> chats,
          String empty,
        ) => _SoloFilterSection(
          key: ValueKey(group),
          group: group,
          label: label,
          chats: chats,
          unread: unreadIds,
          row: row,
          empty: empty,
        );

        final inventorySlivers = <Widget>[
          SliverPadding(
            padding: EdgeInsets.fromLTRB(
              0,
              topSectionHeight + (sliver ? 0 : Grid.xxs),
              0,
              sliver
                  ? 0
                  : visible.isEmpty && sections.isEmpty
                  ? 0
                  : MediaQuery.viewPaddingOf(context).bottom + Grid.gutter,
            ),
            sliver: SliverMainAxisGroup(
              slivers: [
                if (organizationStatus != null)
                  SliverToBoxAdapter(
                    child: Padding(
                      padding: const EdgeInsets.all(Grid.gutter),
                      child: organizationStatus,
                    ),
                  ),
                ?offlineNotice,
                for (final group in sections) ...[
                  if (group != sections.first)
                    const SliverToBoxAdapter(child: SizedBox(height: Grid.xxs)),
                  section(
                    'solo:workspace:${group['id']}',
                    group['name'] as String,
                    visible.where((c) => assignments[c.id] == group['id']),
                    'No chats in this workspace yet',
                  ),
                ],
                if (sections.isNotEmpty)
                  const SliverToBoxAdapter(child: SizedBox(height: Grid.xxs)),
                if (visible.isNotEmpty || sections.isNotEmpty)
                  section(
                    'solo:chats',
                    'Chats',
                    visible.where((c) => !assignments.containsKey(c.id)),
                    'No chats yet',
                  ),
              ],
            ),
          ),
          if (organization != null &&
              visible.isEmpty &&
              sections.isEmpty &&
              !sliver)
            SliverFillRemaining(
              hasScrollBody: false,
              child: Padding(
                padding: EdgeInsets.only(
                  bottom:
                      MediaQuery.viewPaddingOf(context).bottom + Grid.gutter,
                ),
                child: Center(child: _MeEmptyState(onNewChat: newChat)),
              ),
            ),
          if (sliver &&
              organization != null &&
              visible.isEmpty &&
              sections.isEmpty)
            SliverToBoxAdapter(
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.gutter,
                  vertical: Grid.half,
                ),
                child: Text(
                  'No chats yet',
                  style: contentListBodyTextStyle.copyWith(
                    color: context.colors.onSurfaceVariant.withValues(
                      alpha: 0.65,
                    ),
                  ),
                ),
              ),
            ),
        ];
        if (sliver) return SliverMainAxisGroup(slivers: inventorySlivers);
        return BeeRefreshIndicator(
          edgeOffset: topSectionHeight,
          onRefresh: () async {
            ref.invalidate(meGroupsProvider);
            await Future.wait([
              ref.read(channelsProvider.notifier).refresh(),
              if (ref.read(soloFilterProvider) case final manager?)
                manager.refresh(),
            ]);
            await ref.read(meGroupsProvider.future);
          },
          child: CustomScrollView(
            physics: const AlwaysScrollableScrollPhysics(),
            slivers: inventorySlivers,
          ),
        );
      },
    );
    if (sliver) return inventory;
    return FrostedScaffold(
      // New chat owns its keyboard; keep the workspace behind it stationary.
      resizeToAvoidBottomInset: false,
      backgroundColor: context.colors.surface,
      appBar: FrostedAppBar(
        nativeTitle: 'Solo',
        nativeLargeTitle: true,
        automaticallyImplyLeading: false,
        nativeActions: [
          IosNavigationAction(
            label: 'New project',
            icon: BuzzIcons.workspace,
            onPressed: newProject,
          ),
          if (newChatAction == null)
            IosNavigationAction(
              label: 'New chat',
              icon: BuzzIcons.edit,
              onPressed: newChat,
            ),
        ],
        horizontalInset: Grid.gutter,

        title: Text('Solo', style: headerTitleStyle),
        titleStyle: headerTitleStyle,
        actions: [
          ClipRRect(
            borderRadius: BorderRadius.circular(Radii.full),
            child: ColoredBox(
              color: context.controlSurface,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.quarter),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      tooltip: 'New project',
                      icon: const Icon(BuzzIcons.workspace, size: 20),
                      onPressed: newProject,
                    ),
                    if (newChatAction == null)
                      IconButton(
                        tooltip: 'New chat',
                        icon: const Icon(BuzzIcons.edit, size: 20),
                        onPressed: newChat,
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
        bottomHeight: Grid.xxs,
        bottom: const SizedBox.expand(),
      ),
      body: inventory,
    );
  }
}

class _SoloFilterSection extends HookConsumerWidget {
  const _SoloFilterSection({
    super.key,
    required this.group,
    required this.label,
    required this.chats,
    required this.unread,
    required this.row,
    required this.empty,
  });
  final String group;
  final String label;
  final Iterable<Channel> chats;
  final Set<String> unread;
  final SectionChatRowBuilder row;
  final String? empty;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final manager = ref.watch(
      group == 'solo' ? channelFilterProvider : soloFilterProvider,
    );
    useListenable(manager);
    final display = MessagesSectionOptions.active(context)
        ? ref.watch(messagesDisplayProvider)
        : const MessagesDisplay();
    final mode = display.modeFor(manager?.store, group);
    final window = display.windowFor(manager?.store, group);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final selected = ConversationPaneScope.maybeOf(context)?.selectedId;
    final tick = useState(0);
    int? nextExpiry;
    final rows = <Widget>[];
    final sectionSortMode = ref.watch(
      channelSortProvider.select(
        (state) => group == 'solo'
            ? state.sortModeFor(group)
            : state.store.groups[group] ?? ChannelSortMode.recent,
      ),
    );
    final sortMode = display.sort ?? sectionSortMode;
    final limit = display.limitFor(manager?.store, group);
    final orderedChats =
        display.sort != null || sortMode == ChannelSortMode.alpha
        ? sortChannelsForList(chats.toList(), sortMode)
        : chats.toList();
    for (final chat in orderedChats) {
      final activity = mode == ChannelFilterMode.active
          ? ref.watch(channelFilterActivityProvider(chat.id)).asData
          : null;
      final at = activity?.value;
      if (at != null) {
        final expiry = at + window;
        if (expiry > now && (nextExpiry == null || expiry < nextExpiry)) {
          nextExpiry = expiry;
        }
      }
      if (channelMatchesFilter(
        mode: mode,
        unread: unread.contains(chat.id),
        activitySeconds: at,
        activityKnown: activity != null,
        nowSeconds: now,
        windowSeconds: window,
        selected: selected == chat.id,
      )) {
        if (limit == null || rows.length < limit) rows.add(row(chat));
      }
    }
    useEffect(() {
      if (nextExpiry == null) return null;
      return Timer(
        Duration(seconds: nextExpiry - now),
        () => tick.value++,
      ).cancel;
    }, [nextExpiry, tick.value]);
    Widget buildDetail(Channel channel) {
      final viewer = ref.read(myPubkeyProvider);
      final legacy =
          ref.read(meGroupsProvider).value?.channels ?? const <String>[];
      return ChannelDetailPage(
        channel: channel,
        personalChat: channel.isPersonalChat(viewer, legacy),
      );
    }

    if (rows.length < orderedChats.length) {
      rows.add(
        SectionShowMore(
          leadingInset: MessagesSectionOptions.leadingInset(context),
          buildRow: row,
          title: label,
          channels: orderedChats,
          buildDetail: buildDetail,
          liveContent: (routeRef) {
            final organization = routeRef.watch(meGroupsProvider).value;
            final layout = routeRef.watch(sidebarLayoutProvider);
            final list =
                routeRef.watch(channelsProvider).value ?? const <Channel>[];
            final legacy = organization?.channels ?? const <String>[];
            final assignments = organization?.assignments ?? const {};
            final mode =
                (group == 'solo'
                    ? routeRef.watch(messagesDisplayProvider).sort
                    : null) ??
                routeRef.watch(channelSortProvider).store.groups[group] ??
                ChannelSortMode.recent;
            final stars = group == 'solo'
                ? routeRef.watch(channelStarsProvider).store.channels
                : null;
            final viewer = routeRef.watch(myPubkeyProvider);
            return (
              title: label,
              channels: sortChannelsForList(
                list.where((c) {
                  if (!c.isMember || c.isArchived || !c.isSoloChat(legacy)) {
                    return false;
                  }
                  if (group == 'solo') {
                    return layout.sectionKeyFor(
                          c.id,
                          starred: stars?[c.id]?.enabled == true,
                          home: 'solo',
                        ) ==
                        'solo';
                  }
                  return group == 'solo:chats'
                      ? !assignments.containsKey(c.id)
                      : group == 'solo:workspace:${assignments[c.id]}';
                }).toList(),
                mode,
              ),
              buildRow: row,
              buildDetail: (c) => ChannelDetailPage(
                channel: c,
                personalChat: c.isPersonalChat(viewer, legacy),
              ),
            );
          },
        ),
      );
    }
    return MeSection(
      compactHeaderGap: MessagesSectionOptions.active(context),
      label: label,
      sliver: true,
      onFilter: MessagesSectionOptions.active(context)
          ? null
          : () => showSectionViewSheet(
              context,
              group: group,
              title: label,
              sortMode: sortMode,
              onSort: (value) => ref
                  .read(channelSortProvider.notifier)
                  .setSortModeFor(group, value),
            ),
      children: rows.isEmpty
          ? [
              if (chats.isNotEmpty || empty != null)
                _MeSectionEmpty(chats.isEmpty ? empty! : 'No matching chats'),
            ]
          : rows,
    );
  }
}
