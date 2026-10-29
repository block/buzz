import '../../shared/widgets/android_expanding_menu_button.dart';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/community/community_membership_provider.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/community/community_icon_provider.dart';
import '../../shared/community/community_avatar.dart';
import '../../shared/widgets/ios_message_create_button.dart';
import '../../shared/widgets/bottom_up_page_route.dart';
import '../../shared/widgets/mobile_tab_footer_backdrop.dart';
import '../activity/activity_page.dart';
import '../channels/channels_page.dart';
import '../search/search_page.dart';
import 'messages_filters.dart';
import 'messages_navigation_bar.dart';

class HomePage extends HookConsumerWidget {
  const HomePage({
    required this.settingsPageBuilder,
    required this.onAgents,
    required this.defaultAgentBuilder,
    this.communityInvitePageBuilder,
    this.mePageBuilder,
    this.profilePageBuilder,
    this.communityAppearancePageBuilder,
    required this.hasUnreadInbox,
    this.hasUnreadSolo = false,
    super.key,
  });

  final WidgetBuilder settingsPageBuilder;
  final ValueChanged<BuildContext> onAgents;
  final WidgetBuilder defaultAgentBuilder;
  final WidgetBuilder? profilePageBuilder;

  /// Builds a sliver inventory and publishes its two creation actions.
  final Widget Function(
    BuildContext,
    ValueNotifier<VoidCallback?>,
    ValueNotifier<VoidCallback?>,
  )?
  mePageBuilder;
  final WidgetBuilder? communityInvitePageBuilder;
  final WidgetBuilder? communityAppearancePageBuilder;
  final bool hasUnreadInbox;
  final bool hasUnreadSolo;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final route = ModalRoute.of(context);
    final coveringProgress = useAnimation(
      route?.secondaryAnimation ?? kAlwaysDismissedAnimation,
    );
    final nativeControlsVisible =
        (route?.isCurrent ?? true) && coveringProgress == 0;

    final communitySurfaceProgress = useState(0.0);
    final recordingFocus = useState(false);
    final onRecordingChanged = useCallback((bool value) {
      if (context.mounted) recordingFocus.value = value;
    }, const []);
    final filter = useState('All');
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
        builder: (_) => const SearchPage(),
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
    final messages = ChannelsPage(
      settingsPageBuilder: settingsPageBuilder,
      hasUnreadInbox: hasUnreadInbox,
      onInbox: () => Navigator.of(context).push(
        BottomUpPageRoute<void>(
          builder: (_) => const ActivityPage(),
          reducedMotion: reducedMotion,
        ),
      ),
      messagesActionsBuilder: (openSwitcher) {
        final moreActions = [
          (
            label: 'Switch workspace',
            symbol: 'arrow.left.arrow.right',
            onPressed: openSwitcher,
          ),
          if (canInvite && communityInvitePageBuilder != null)
            (
              label: 'Invite',
              symbol: 'person.badge.plus',
              onPressed: () => openPage(communityInvitePageBuilder!),
            ),
          if (communityAppearancePageBuilder != null)
            (
              label: 'Appearance',
              symbol: 'circle.lefthalf.filled',
              onPressed: () => openPage(communityAppearancePageBuilder!),
            ),
          (
            label: 'Agents',
            symbol: 'person.2',
            onPressed: () => onAgents(context),
          ),
          (label: 'Settings', symbol: 'gearshape', onPressed: openSettings),
        ];
        return [
          if (defaultTargetPlatform == TargetPlatform.iOS &&
              !recordingFocus.value)
            IosMessageCreateButton(
              actions: moreActions,
              dividerBefore: const {1},
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
          else
            AndroidExpandingMenuButton(
              label: 'Community menu',
              actions: moreActions,
              dividerBefore: const {1},
              width: 48,
              height: 48,
              trigger: CommunityAvatar(
                name: communityName,
                relayUrl: community?.relayUrl,
                size: 36,
              ),
            ),
        ];
      },
      messagesFilters: MessagesFilters(
        useNativeGlass: !recordingFocus.value,
        hasUnreadSolo: hasUnreadSolo,
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
    final bottom =
        MediaQuery.viewPaddingOf(context).bottom > mobileTabBarBottomGap
        ? MediaQuery.viewPaddingOf(context).bottom
        : mobileTabBarBottomGap;
    return Scaffold(
      resizeToAvoidBottomInset: false,
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
              child: ExcludeSemantics(
                excluding: recordingFocus.value,
                child: messages,
              ),
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
          Positioned.fill(
            child: IgnorePointer(
              ignoring: !recordingFocus.value,
              child: BackdropFilter(
                key: const ValueKey('voice-recording-focus'),
                enabled: recordingFocus.value,
                filter: ui.ImageFilter.blur(sigmaX: 6, sigmaY: 6),
                child: ColoredBox(
                  color: recordingFocus.value
                      ? context.colors.surface.withValues(alpha: 0.18)
                      : Colors.transparent,
                ),
              ),
            ),
          ),
          if (defaultTargetPlatform != TargetPlatform.iOS ||
              nativeControlsVisible)
            Positioned(
              left: 0,
              right: 0,
              bottom: bottom,
              child: Center(
                child: MessagesNavigationBar(
                  onSearch: openSearch,
                  onRecordingChanged: onRecordingChanged,
                  agent: defaultAgentBuilder(context),
                  creationActions: [
                    (
                      label: 'New chat',
                      symbol: 'square.and.pencil',
                      onPressed: newChat.value,
                    ),
                    (
                      label: 'New workspace',
                      symbol: 'folder.badge.plus',
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
              ),
            ),
        ],
      ),
    );
  }
}
