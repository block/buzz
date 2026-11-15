import 'dart:convert';

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
import '../../activity/compose_drafts_provider.dart';
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
import '../../profile/user_profile_sheet.dart';
import '../channel_actions_sheet.dart';
import '../../../shared/share/share_suggestions.dart';
import '../channel_detail_page.dart';
import '../channel_identity_names_provider.dart';
import '../channel_member_profile_actions.dart';
import '../channel_management_provider.dart';
import '../channels_provider.dart';

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
    final viewer = ref.watch(myPubkeyProvider);
    final offlineSaveFailed =
        ref.watch(meOfflineSaveWarningProvider).value ?? false;
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

    final VoidCallback? newProject = busy.value || !groups.hasValue
        ? null
        : () => perform(() async {
            await createMeProject(context, ref);
          });
    final VoidCallback? newChat = busy.value || !groups.hasValue
        ? null
        : () => perform(() => _openNewMeChat(context, ref));

    useEffect(() {
      var active = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (active) {
          newChatAction?.value = newChat;
          newWorkspaceAction?.value = newProject;
        }
      });
      return () => active = false;
    }, [newChatAction, newWorkspaceAction, busy.value, groups.hasValue]);

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
        Widget row(Channel channel) {
          final pane = ConversationPaneScope.maybeOf(context);
          final selected = pane?.selectedId == channel.id;
          return DecoratedBox(
            decoration: BoxDecoration(
              color: selected ? context.colors.primaryContainer : null,
              borderRadius: BorderRadius.circular(Radii.md),
            ),
            child: Semantics(
              selected: selected,
              child: _MeChatRow(
                channelId: channel.id,
                name: channel.name,
                isUnread: unreadIds.contains(channel.id),
                onLongPress: () => showChannelDetailsSheet(
                  context: context,
                  channel: channel,
                  currentPubkey: viewer,
                  onMemberTap: (context, pubkey) => showUserProfileSheet(
                    context,
                    pubkey,
                    names: channelIdentityNamesProvider(channel.id),
                    contextualActions: (_) => ChannelMemberProfileActions(
                      channel: channel,
                      pubkey: pubkey,
                    ),
                  ),
                  personalChat: true,
                ),
                onTap: () => openConversationPane(
                  context,
                  channel.id,
                  (_) => ChannelDetailPage(
                    channel: channel,
                    personalChat: channel.isPersonalChat(
                      viewer,
                      legacyChannels,
                    ),
                  ),
                ),
              ),
            ),
          );
        }

        final inventorySlivers = <Widget>[
          SliverPadding(
            padding: EdgeInsets.fromLTRB(
              Grid.gutter - Grid.xxs,
              topSectionHeight + (sliver ? 0 : Grid.xxs),
              Grid.gutter - Grid.xxs,
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
                if (offlineSaveFailed)
                  SliverToBoxAdapter(
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
                  ),
                for (final group in sections) ...[
                  if (group != sections.first)
                    const SliverToBoxAdapter(child: SizedBox(height: Grid.xxs)),
                  MeSection(
                    sliver: true,
                    key: ValueKey(group['id']),
                    label: group['name'] as String,
                    children: [
                      for (final channel in visible.where(
                        (c) => assignments[c.id] == group['id'],
                      ))
                        row(channel),
                      if (!visible.any((c) => assignments[c.id] == group['id']))
                        const _MeSectionEmpty('No chats in this workspace yet'),
                    ],
                  ),
                ],
                if (sections.isNotEmpty)
                  const SliverToBoxAdapter(child: SizedBox(height: Grid.xxs)),
                if (visible.isNotEmpty || sections.isNotEmpty)
                  MeSection(
                    sliver: true,
                    key: const ValueKey('me-ungrouped-chats'),
                    label: 'Chats',
                    children: [
                      for (final channel in visible.where(
                        (c) => !assignments.containsKey(c.id),
                      ))
                        row(channel),
                      if (!visible.any((c) => !assignments.containsKey(c.id)))
                        const _MeSectionEmpty('No chats yet'),
                    ],
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
            await ref.read(channelsProvider.notifier).refresh();
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
