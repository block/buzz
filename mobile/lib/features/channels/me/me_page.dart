import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import '../../../shared/profile/user_cache_provider.dart';
import '../../../shared/animated_avatar.dart';
import '../../../shared/widgets/app_list.dart';
import '../../../shared/widgets/concentric_sheet_surface.dart';
import '../../../shared/widgets/bee_refresh_indicator.dart';
import 'me_assistant_list.dart';
import 'me_section.dart';
import 'me_unread_provider.dart';
import '../../../shared/widgets/avatar_image.dart';
import '../../../shared/me/me_groups.dart';
import '../../../shared/me/me_chat_creation.dart';
import '../../../shared/me/me_project_creation.dart';
import '../../../shared/me/me_sync.dart';
import '../../../shared/me/me_recovery.dart';
import '../../../shared/me/me_assistants.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/frosted_app_bar.dart';
import '../../../shared/widgets/frosted_scaffold.dart';
import '../../../shared/widgets/buzz_loading_indicator.dart';
import '../../../shared/widgets/modal_presentation.dart';
import '../channel.dart';
import '../channel_detail_page.dart';
import '../channel_management_provider.dart';
import '../channels_provider.dart';

part 'me_page_helpers.dart';
part 'me_empty_state.dart';
part 'new_me_chat.dart';
part 'native_new_me_chat.dart';
part 'new_me_project.dart';

/// Personal chats and projects shared with the desktop Me workspace.
class MePage extends HookConsumerWidget {
  const MePage({this.newChatAction, super.key});

  /// Publishes the current chat action to the persistent Home footer.
  final ValueNotifier<VoidCallback?>? newChatAction;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
      color: navigationPrimaryForeground(context),
    );
    final topSectionHeight = frostedAppBarHeight(
      context,
      nativeLargeTitle: true,
      titleStyle: headerTitleStyle,
      bottomHeight: Grid.xxs,
    );
    // Warm the directory while browsing Me, before the creation form opens.
    final assistants = ref.watch(meAssistantsProvider).value;
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
    final syncClosed = ref.watch(meSyncWarningProvider).value ?? false;
    final channels = ref.watch(channelsProvider);
    final unreadIds = ref.watch(meUnreadChannelIdsProvider);
    final busy = useState(false);
    Future<void> perform(Future<void> Function() action) async {
      if (busy.value) return;
      busy.value = true;
      try {
        await action();
      } catch (error) {
        if (error is MeRecoveryConflict && context.mounted) {
          await _offerMeRecovery(context, ref.read(meSyncProvider));
        }
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(error.toString().replaceFirst('Bad state: ', '')),
            ),
          );
        }
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    final VoidCallback? newProject = busy.value || !groups.hasValue
        ? null
        : () => perform(() async {
            await _createMeProject(context, ref);
          });
    final VoidCallback? newChat = busy.value || !groups.hasValue
        ? null
        : () => perform(() async {
            final result = await _showNewMeChat(context, ref);
            if (result == null || !context.mounted) return;
            await Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => ChannelDetailPage(
                  channel: result.$1,
                  personalChat: true,
                  initialAssistant: result.$2,
                ),
              ),
            );
          });

    useEffect(() {
      var active = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (active) newChatAction?.value = newChat;
      });
      return () => active = false;
    }, [newChatAction, busy.value, groups.hasValue]);

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
            symbol: 'folder.badge.plus',
            icon: BuzzIcons.folderPlus,
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
        showBottomDivider: true,
        bottomDividerOpacity: 0.07,
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
                      icon: const Icon(BuzzIcons.folderPlus, size: 20),
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
      body: groups.when(
        loading: () => const Center(child: BuzzLoadingIndicator()),
        error: (_, _) =>
            _MeRetry(onRetry: () => ref.invalidate(meGroupsProvider)),
        data: (organization) => channels.when(
          loading: () => const Center(child: BuzzLoadingIndicator()),
          error: (_, _) =>
              _MeRetry(onRetry: () => ref.invalidate(channelsProvider)),
          data: (all) {
            final visible =
                all
                    .where(
                      (c) =>
                          c.isMember &&
                          !c.isArchived &&
                          organization.channels.contains(c.id),
                    )
                    .toList()
                  ..sort(
                    (a, b) => (b.lastMessageAt ?? b.createdAt).compareTo(
                      a.lastMessageAt ?? a.createdAt,
                    ),
                  );
            Widget row(Channel channel) => _MeChatRow(
              name: channel.name,
              isUnread: unreadIds.contains(channel.id),
              onTap: () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) =>
                      ChannelDetailPage(channel: channel, personalChat: true),
                ),
              ),
            );
            return BeeRefreshIndicator(
              edgeOffset: topSectionHeight,
              onRefresh: () async {
                ref.invalidate(meGroupsProvider);
                await ref.read(channelsProvider.notifier).refresh();
                await ref.read(meGroupsProvider.future);
              },
              child: CustomScrollView(
                physics: const AlwaysScrollableScrollPhysics(),
                slivers: [
                  SliverPadding(
                    padding: EdgeInsets.fromLTRB(
                      Grid.gutter - Grid.xxs,
                      topSectionHeight + Grid.xxs,
                      Grid.gutter - Grid.xxs,
                      organization.channels.isEmpty &&
                              organization.groups.isEmpty
                          ? 0
                          : MediaQuery.viewPaddingOf(context).bottom +
                                Grid.gutter,
                    ),
                    sliver: SliverList.list(
                      children: [
                        if (syncClosed)
                          Semantics(
                            liveRegion: true,
                            child: Column(
                              children: [
                                const Text(
                                  'Live updates stopped. Your saved chats are still available.',
                                ),
                                TextButton(
                                  onPressed: () =>
                                      ref.invalidate(meGroupsProvider),
                                  child: const Text('Retry live updates'),
                                ),
                              ],
                            ),
                          ),
                        if (offlineSaveFailed)
                          Semantics(
                            liveRegion: true,
                            child: Column(
                              children: [
                                const Text(
                                  'Your chats are available, but couldn’t be saved for offline use.',
                                ),
                                TextButton(
                                  onPressed: () =>
                                      ref.invalidate(meGroupsProvider),
                                  child: const Text('Retry offline save'),
                                ),
                              ],
                            ),
                          ),
                        for (final group in organization.groups)
                          MeSection(
                            key: ValueKey(group['id']),
                            icon: BuzzIcons.folder,
                            label: group['name'] as String,
                            hasUnread: visible.any(
                              (c) =>
                                  organization.assignments[c.id] ==
                                      group['id'] &&
                                  unreadIds.contains(c.id),
                            ),
                            children: [
                              for (final channel in visible.where(
                                (c) =>
                                    organization.assignments[c.id] ==
                                    group['id'],
                              ))
                                row(channel),
                              if (!visible.any(
                                (c) =>
                                    organization.assignments[c.id] ==
                                    group['id'],
                              ))
                                const ListTile(
                                  title: Text('No chats in this project'),
                                ),
                            ],
                          ),
                        if (organization.channels.isNotEmpty ||
                            organization.groups.isNotEmpty)
                          MeSection(
                            key: const ValueKey('me-ungrouped-chats'),
                            icon: BuzzIcons.messageCircle,
                            label: 'Chats',
                            hasUnread: visible.any(
                              (c) =>
                                  !organization.assignments.containsKey(c.id) &&
                                  unreadIds.contains(c.id),
                            ),
                            children: [
                              for (final channel in visible.where(
                                (c) =>
                                    !organization.assignments.containsKey(c.id),
                              ))
                                row(channel),
                              if (!visible.any(
                                (c) =>
                                    !organization.assignments.containsKey(c.id),
                              ))
                                const ListTile(title: Text('No chats yet')),
                            ],
                          ),
                      ],
                    ),
                  ),
                  if (organization.channels.isEmpty &&
                      organization.groups.isEmpty)
                    SliverFillRemaining(
                      hasScrollBody: false,
                      child: Padding(
                        padding: EdgeInsets.only(
                          bottom:
                              MediaQuery.viewPaddingOf(context).bottom +
                              Grid.gutter,
                        ),
                        child: Center(child: _MeEmptyState(onNewChat: newChat)),
                      ),
                    ),
                ],
              ),
            );
          },
        ),
      ),
    );
  }
}
