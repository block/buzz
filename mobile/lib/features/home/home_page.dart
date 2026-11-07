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
import 'messages_filters.dart';
import 'messages_navigation_bar.dart';
import '../../shared/widgets/covered_route_keyboard_insets.dart';

class HomePage extends HookConsumerWidget {
  const HomePage({
    required this.settingsPageBuilder,
    required this.onAgents,
    required this.defaultAgentBuilder,
    this.communityInvitePageBuilder,
    this.mePageBuilder,
    this.communityAppearancePageBuilder,
    super.key,
  });

  final WidgetBuilder settingsPageBuilder;
  final ValueChanged<BuildContext> onAgents;
  final WidgetBuilder defaultAgentBuilder;

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
        builder: (_) => const CoveredRouteKeyboardInsets(child: SearchPage()),
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
        builder: (_) => const ActivityPage(),
        reducedMotion: reducedMotion,
      ),
    );
    final messages = ChannelsPage(
      settingsPageBuilder: settingsPageBuilder,
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
    return Scaffold(
      resizeToAvoidBottomInset: false,
      // Register the floating controls with Scaffold so snackbars clear them.
      extendBody: true,
      bottomNavigationBar: Padding(
        padding: EdgeInsets.only(bottom: bottom),
        child: SizedBox(
          height: mobileTabBarHeight,
          child:
              defaultTargetPlatform != TargetPlatform.iOS ||
                  nativeControlsVisible
              ? Center(
                  child: MessagesNavigationBar(
                    onSearch: openSearch,
                    agent: defaultAgentBuilder(context),
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
                      (
                        label: 'New DM',
                        symbol: 'bubble.left',
                        onPressed: newDm.value,
                      ),
                      (
                        label: 'New channel',
                        symbol: 'number',
                        onPressed: newChannel.value,
                      ),
                    ],
                  ),
                )
              : null,
        ),
      ),
      body: Stack(
        children: [
          MediaQuery(
            data: MediaQuery.of(context).copyWith(
              padding: MediaQuery.paddingOf(context).copyWith(
                bottom:
                    MediaQuery.paddingOf(context).bottom +
                    mobileTabBarHeight +
                    mobileTabBarBottomGap,
              ),
              viewPadding: MediaQuery.viewPaddingOf(context).copyWith(
                bottom:
                    MediaQuery.viewPaddingOf(context).bottom +
                    mobileTabBarHeight +
                    mobileTabBarBottomGap,
              ),
            ),
            child: Opacity(
              key: const Key('home-community-surface-visibility'),
              // The switcher uses a transparent route for its avatar flight.
              // Keep Home laid out, but do not paint it through UIKit glass.
              opacity: communitySurfaceProgress.value == 0 ? 1 : 0,
              child: messages,
            ),
          ),
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
    );
  }
}
