import 'package:duo_kit/duo_kit.dart';

import '../../shared/widgets/adaptive_conversation_pane.dart';
import '../../shared/widgets/duo_workspace_scaffold.dart';
import '../../shared/widgets/duo_toolbar.dart';
import '../../shared/widgets/ios_navigation_metrics.dart';
import '../../shared/widgets/android_expanding_menu_button.dart';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/organization_environment.dart';
import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/community/community_membership_provider.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/community/community_icon_provider.dart';
import '../../shared/community/community_avatar.dart';
import '../../shared/widgets/ios_message_create_button.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/bottom_up_page_route.dart';
import '../../shared/widgets/mobile_tab_footer_backdrop.dart';
import '../activity/activity_page.dart';
import '../activity/inbox_unread_provider.dart';
import '../channels/channels_page.dart';
import '../search/search_page.dart';
import 'home_screen_shortcut_host.dart';
import 'messages_filters.dart';
import 'messages_navigation_bar.dart';
import '../../shared/widgets/covered_route_keyboard_insets.dart';

class HomePage extends HookConsumerWidget {
  const HomePage({
    required this.settingsPageBuilder,
    required this.onAgents,
    required this.defaultAgentBuilder,
    required this.openBestieConversation,
    this.communityInvitePageBuilder,
    this.mePageBuilder,
    this.communityAppearancePageBuilder,
    super.key,
  });

  final WidgetBuilder settingsPageBuilder;
  final ValueChanged<BuildContext> onAgents;
  final WidgetBuilder defaultAgentBuilder;

  /// Opens the direct conversation with the Bestie, for the app shortcut.
  final Future<void> Function(BuildContext, WidgetRef, String pubkey)
  openBestieConversation;

  /// Builds a sliver inventory and publishes its two creation actions.
  final Widget Function(
    BuildContext,
    ValueNotifier<VoidCallback?>,
    ValueNotifier<VoidCallback?>,
  )?
  mePageBuilder;
  final WidgetBuilder? communityInvitePageBuilder;
  final WidgetBuilder? communityAppearancePageBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communityId = ref.watch(
      activeCommunityProvider.select((value) => value.value?.id),
    );
    final conversations = useMemoized(ConversationPaneController.new, [
      communityId,
    ]);
    useEffect(() => conversations.dispose, [conversations]);
    useListenable(conversations);
    final strip = FoldGeometry.of(context).strip;
    final wide = usesConversationPanes(context, strip: strip);
    final sidebarWidth =
        defaultTargetPlatform == TargetPlatform.iOS && strip != null
        ? 375.0
        : null;
    final showWorkspaceNavigation = wide || conversations.selectedId == null;
    final route = ModalRoute.of(context);
    final coveringProgress = useAnimation(
      route?.secondaryAnimation ?? kAlwaysDismissedAnimation,
    );
    final nativeControlsVisible =
        (route?.isCurrent ?? true) && coveringProgress == 0;

    final communitySurfaceProgress = useState(0.0);
    final filter = useState('All');
    final hasUnreadInbox = ref.watch(inboxHasUnreadProvider);
    final inboxLabel = hasUnreadInbox ? 'Activity, unread items' : 'Activity';
    final reselect = useValueNotifier(0);
    final newChat = useValueNotifier<VoidCallback?>(null);
    final newWorkspace = useValueNotifier<VoidCallback?>(null);
    final newChannel = useValueNotifier<VoidCallback?>(null);
    final newDm = useValueNotifier<VoidCallback?>(null);
    useListenable(newChat);
    useListenable(newWorkspace);
    useListenable(newChannel);
    useListenable(newDm);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    void openSearch() => Navigator.of(context).push(
      BottomUpPageRoute<void>(
        builder: (_) => CoveredRouteKeyboardInsets(
          child: AdaptiveConversationDestination(
            preferredListWidth: sidebarWidth,
            listToolbarSlots: 1,
            child: const SearchPage(),
          ),
        ),
        reducedMotion: reducedMotion,
      ),
    );
    void openSettings() => Navigator.of(
      context,
    ).push(MaterialPageRoute<void>(builder: settingsPageBuilder));
    final community = ref.watch(activeCommunityProvider).unwrapPrevious().value;
    final communityName = community?.name.trim() ?? '';
    final communityIcon = community == null
        ? null
        : ref.watch(communityIconPresentationProvider(community.relayUrl));
    final role = ref.watch(currentCommunityRoleProvider).unwrapPrevious();
    final canInvite = role.hasError || canManageCommunityInvites(role.value);
    void openPage(WidgetBuilder builder) =>
        Navigator.of(context).push(MaterialPageRoute<void>(builder: builder));
    final solo = mePageBuilder?.call(context, newChat, newWorkspace);
    void openInbox() => Navigator.of(context).push(
      BottomUpPageRoute<void>(
        builder: (_) => AdaptiveConversationDestination(
          preferredListWidth: sidebarWidth,
          child: const ActivityPage(),
        ),
        reducedMotion: reducedMotion,
      ),
    );
    final messages = ChannelsPage(
      settingsPageBuilder: settingsPageBuilder,
      messagesActionsWidth:
          2 * (defaultTargetPlatform == TargetPlatform.iOS ? 44 : 48) + Grid.xs,
      messagesActionsBuilder: (openSwitcher, avatarKey) {
        final moreActions = [
          // An internal build reaches only its organization's community.
          if (!OrganizationEnvironment.enabled)
            (
              label: 'Switch community',
              symbol: 'arrow.left.arrow.right',
              onPressed: openSwitcher,
            ),
          if (canInvite && communityInvitePageBuilder != null)
            (
              label: 'Invite',
              symbol: 'person.badge.plus',
              onPressed: () => openPage(communityInvitePageBuilder!),
            ),
          (
            label: 'Agents',
            symbol: 'agent',
            onPressed: () => onAgents(context),
          ),
          if (communityAppearancePageBuilder != null)
            (
              label: 'Appearance',
              symbol: 'circle.lefthalf.filled',
              onPressed: () => openPage(communityAppearancePageBuilder!),
            ),
          (label: 'Settings', symbol: 'gearshape', onPressed: openSettings),
        ];
        return [
          if (defaultTargetPlatform == TargetPlatform.iOS &&
              !nativeControlsVisible)
            const SizedBox(width: 44, height: 44)
          else
            Stack(
              alignment: Alignment.center,
              children: [
                defaultTargetPlatform == TargetPlatform.iOS
                    ? IosGlassNavigationButton(
                        icon: IosGlassNavigationIcon.bell,
                        semanticLabel: inboxLabel,
                        width: 44,
                        height: 44,
                        controlSize: 44,
                        foregroundColor: context.colors.onSurface,
                        onPressed: openInbox,
                      )
                    : IconButton.filledTonal(
                        tooltip: inboxLabel,
                        style: IconButton.styleFrom(
                          backgroundColor:
                              context.colors.surfaceContainerHighest,
                          foregroundColor: context.colors.onSurface,
                          minimumSize: const Size.square(48),
                        ),
                        icon: const Icon(BuzzIcons.bell),
                        onPressed: openInbox,
                      ),
                Positioned.fill(
                  child: IgnorePointer(
                    child: ExcludeSemantics(
                      child: Center(
                        child: AnimatedOpacity(
                          opacity: hasUnreadInbox ? 1 : 0,
                          duration: reducedMotion
                              ? Duration.zero
                              : const Duration(milliseconds: 150),
                          curve: Curves.easeOut,
                          child: SizedBox.square(
                            key: const Key('home-inbox-badge'),
                            dimension: 24,
                            child: Stack(
                              clipBehavior: Clip.none,
                              children: [
                                Positioned(
                                  top: 0,
                                  right: 2,
                                  child: Container(
                                    key: const Key('home-inbox-dot'),
                                    width: 10,
                                    height: 10,
                                    decoration: BoxDecoration(
                                      shape: BoxShape.circle,
                                      color: context.colors.primary,
                                      border: Border.all(
                                        color:
                                            defaultTargetPlatform ==
                                                TargetPlatform.android
                                            ? context
                                                  .colors
                                                  .surfaceContainerHighest
                                            : context.controlSurface,
                                        width: 2,
                                      ),
                                    ),
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          const SizedBox(width: Grid.xs),
          SizedBox(
            key: avatarKey,
            child: defaultTargetPlatform == TargetPlatform.iOS
                ? IosMessageCreateButton(
                    actions: moreActions,
                    dividerBefore: OrganizationEnvironment.enabled
                        ? const {}
                        : const {1},
                    icon: 'more',
                    avatarUrl: communityIcon,
                    avatarInitial: communityName.isEmpty
                        ? '?'
                        : communityName.characters.first.toUpperCase(),
                    accessibilityLabel: 'Community menu',
                    width: 44,
                    height: 44,
                    plain: false,
                  )
                : AndroidExpandingMenuButton(
                    label: 'Community menu',
                    backgroundColor: context.colors.surfaceContainerHighest,
                    actions: moreActions,
                    dividerBefore: OrganizationEnvironment.enabled
                        ? const {}
                        : const {1},
                    width: 48,
                    height: 48,
                    trigger: CommunityAvatar(
                      name: communityName,
                      relayUrl: community?.relayUrl,
                      size: 36,
                    ),
                  ),
          ),
        ];
      },
      messagesFilters: (isCollapsed) => MessagesFilters(
        isCollapsed: isCollapsed,
        selected: filter.value,
        onSelected: (value) {
          filter.value = value;
          reselect.value++;
        },
      ),
      soloSliver: solo == null
          ? null
          : SliverOffstage(
              key: const ValueKey('messages-solo-inventory'),
              offstage: filter.value == 'Team',
              sliver: solo,
            ),
      showTeam: filter.value != 'Solo',
      showTeamHeading: filter.value == 'All',
      newChannelAction: newChannel,
      newDmAction: newDm,
      communityInvitePageBuilder: communityInvitePageBuilder,
      communityAppearancePageBuilder: communityAppearancePageBuilder,
      tabReselection: reselect,
      onSettingsTransitionProgress: (progress) {
        communitySurfaceProgress.value = progress;
      },
    );
    final bottom = mobileTabBarBottom(context);
    Widget navigationBar(Axis axis) => MessagesNavigationBar(
      axis: axis,
      onSearch: openSearch,
      agent: ConversationPaneScope(
        controller: conversations,
        child: Builder(builder: defaultAgentBuilder),
      ),
      creationActions: [
        (
          label: 'New chat',
          symbol: 'square.and.pencil',
          onPressed: newChat.value,
        ),
        (
          label: 'New workspace',
          symbol: 'workspace',
          onPressed: newWorkspace.value,
        ),
        (label: 'New DM', symbol: 'bubble.left', onPressed: newDm.value),
        (label: 'New channel', symbol: 'number', onPressed: newChannel.value),
      ],
    );
    return DuoWorkspaceScaffold(
      navigationBuilder: (context, placement) =>
          wide ||
              !showWorkspaceNavigation ||
              !nativeControlsVisible ||
              communitySurfaceProgress.value != 0
          ? const SizedBox.shrink()
          : Transform.translate(
              offset: Offset(
                IosNavigationMetrics.of(context).railCenter(
                      MediaQuery.sizeOf(context),
                      strip!.rect.center.dx,
                    ) -
                    strip.rect.center.dx,
                0,
              ),
              child: SafeArea(
                bottom: false,
                child: Padding(
                  // Match the compact composer's safe-area gutter and its
                  // downward resting offset in _ComposerDockFrame.
                  padding: EdgeInsets.only(
                    bottom:
                        (MediaQuery.viewPaddingOf(context).bottom +
                                Grid.xxs -
                                Grid.twelve -
                                Grid.quarter)
                            .clamp(0.0, double.infinity),
                  ),
                  child: Align(
                    alignment: Alignment.bottomCenter,
                    child: navigationBar(Axis.vertical),
                  ),
                ),
              ),
            ),
      body: Scaffold(
        resizeToAvoidBottomInset: false,
        extendBody: true,
        bottomNavigationBar: strip == null && showWorkspaceNavigation
            ? Padding(
                padding: EdgeInsets.only(bottom: bottom),
                child: SizedBox(
                  height: mobileTabBarHeight,
                  child:
                      (defaultTargetPlatform != TargetPlatform.iOS ||
                          nativeControlsVisible)
                      ? Center(child: navigationBar(Axis.horizontal))
                      : null,
                ),
              )
            : null,
        body: Stack(
          children: [
            if (defaultTargetPlatform == TargetPlatform.iOS)
              ConversationPaneScope(
                controller: conversations,
                child: HomeScreenShortcutHost(
                  newChat: newChat,
                  newDm: newDm,
                  onAgents: onAgents,
                  openBestieConversation: openBestieConversation,
                ),
              ),
            MediaQueryOverride(
              transform: (media) => media.copyWith(
                padding: media.padding.copyWith(
                  bottom:
                      media.padding.bottom +
                      (strip == null && showWorkspaceNavigation
                          ? mobileTabBarHeight + mobileTabBarBottomGap
                          : 0),
                ),
                viewPadding: media.viewPadding.copyWith(
                  bottom:
                      media.viewPadding.bottom +
                      (strip == null && showWorkspaceNavigation
                          ? mobileTabBarHeight + mobileTabBarBottomGap
                          : 0),
                ),
              ),
              child: Opacity(
                key: const Key('home-community-surface-visibility'),
                // The switcher uses a transparent route for its avatar flight.
                // Keep Home laid out, but do not paint it through UIKit glass.
                opacity: communitySurfaceProgress.value == 0 ? 1 : 0,
                child: AdaptiveConversationPane(
                  controller: conversations,
                  strip: strip,
                  preferredListWidth: sidebarWidth,
                  emptyContent: strip == null
                      ? null
                      : Image.asset(
                          'assets/images/buzz-icon.png',
                          width: 80,
                          color: Theme.of(context).colorScheme.onSurfaceVariant,
                          colorBlendMode: BlendMode.srcIn,
                          semanticLabel: 'Buzz',
                        ),
                  list: wide && strip != null
                      ? Builder(
                          builder: (paneContext) {
                            final paneMedia = MediaQuery.of(paneContext);
                            final insets =
                                DuoToolbarScope.maybeOf(
                                  paneContext,
                                )?.contentInsets ??
                                EdgeInsets.zero;
                            return Stack(
                              fit: StackFit.expand,
                              children: [
                                MediaQuery(
                                  data: paneMedia.copyWith(
                                    padding: paneMedia.padding.copyWith(
                                      bottom:
                                          paneMedia.padding.bottom +
                                          mobileTabBarHeight +
                                          mobileTabBarBottomGap,
                                    ),
                                    viewPadding: paneMedia.viewPadding.copyWith(
                                      bottom:
                                          paneMedia.viewPadding.bottom +
                                          mobileTabBarHeight +
                                          mobileTabBarBottomGap,
                                    ),
                                  ),
                                  child: messages,
                                ),
                                Positioned(
                                  left: 0,
                                  right: 0,
                                  bottom: 0,
                                  child: IgnorePointer(
                                    child: MobileTabFooterBackdrop(
                                      height: mobileTabFooterBackdropHeight(
                                        paneContext,
                                      ),
                                      tint: paneContext.controlSurface,
                                    ),
                                  ),
                                ),
                                if (nativeControlsVisible)
                                  Positioned(
                                    left: insets.left,
                                    right: insets.right,
                                    bottom: mobileTabBarBottom(paneContext),
                                    height: mobileTabBarHeight,
                                    child: Center(
                                      child: navigationBar(Axis.horizontal),
                                    ),
                                  ),
                              ],
                            );
                          },
                        )
                      : messages,
                ),
              ),
            ),
            if (strip == null && showWorkspaceNavigation)
              Positioned(
                left: 0,
                right: 0,
                bottom: 0,
                child: IgnorePointer(
                  child: MobileTabFooterBackdrop(
                    height: mobileTabFooterBackdropHeight(context),
                    tint: context.controlSurface,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
