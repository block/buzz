import 'dart:async';
import 'dart:io';
import 'dart:math' show max, min, pi;
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/physics.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/auth/auth.dart';
import '../../shared/community/community_icon_provider.dart';
import '../../shared/community/community_membership_provider.dart';
import '../../shared/widgets/app_list_card_item.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/anchored_popover_menu.dart';
import '../../shared/widgets/bee_refresh_indicator.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/concentric_sheet_surface.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/ios_navigation_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/confirmation_dialog.dart';
import '../../shared/widgets/skeleton.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../profile/profile_avatar.dart';
import '../profile/profile_provider.dart';
import '../profile/presence_cache_provider.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../pairing/pairing_page.dart';
import '../pairing/pairing_provider.dart';
import 'channel.dart';
import 'channel_actions_sheet.dart';
import 'channel_detail_page.dart';
import 'channel_management_provider.dart';
import 'dm_channel_labels.dart';
import 'ephemeral_channel_display.dart';
import 'channel_mutes/channel_mutes_provider.dart';
import 'channel_sections/channel_sections_provider.dart';
import 'channel_sections/channel_sections_storage.dart';
import 'channel_sort/channel_sort_provider.dart';
import 'channel_sort/channel_sort_storage.dart';
import 'channel_stars/channel_stars_provider.dart';
import 'channels_provider.dart';
import '../../shared/read_state/deferred_read_state_update.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/read_state/read_state_time.dart';
import 'unread_badge/observed_unread_event.dart';

part 'channels_page/body.dart';
part 'channels_page/browse_channels_sheet.dart';
part 'channels_page/sections.dart';
part 'channels_page/channel_tile.dart';
part 'channels_page/sheets.dart';
part 'channels_page/badges.dart';
part 'channels_page/skeleton.dart';
part 'channels_page/community.dart';
part 'channels_page/community_switcher.dart';
part 'channels_page/community_switcher_action.dart';
part 'channels_page/quick_actions.dart';
part 'channels_page/quick_actions_launcher.dart';

enum _QuickAction { createChannel, newDm, browseChannels }

const double _kChannelSectionInset = Grid.gutter;
const double _kChannelLeadingWidth = 22.0;
const double _kChannelIconSize = 18.0;
const double _kChannelLabelGap = Grid.xxs;
const double _kChannelRowVerticalPadding = Grid.xxs + Grid.quarter;
const double _kSectionSpacingTightening = Grid.half;
const double _kSectionHeaderVerticalPadding =
    _kChannelRowVerticalPadding - _kSectionSpacingTightening;
// Section headers include touch targets for their actions, so their visual
// centre sits lower than a channel row's. This keeps an expanded section's
// final row equally spaced from the following divider.
const double _kExpandedSectionTrailingPadding =
    11.0 - _kSectionSpacingTightening;
const double _kChannelLabelInset =
    _kChannelSectionInset + _kChannelLeadingWidth + _kChannelLabelGap;

/// DM avatars are circles, so they fill their box edge to edge where a channel
/// glyph leaves 4dp of slack inside the same 22dp leading column. Sizing them to
/// the glyph's ink width keeps the icon-to-label distance identical across both
/// sections while the labels stay on [_kChannelLabelInset].
const double _kDmAvatarSize = _kChannelIconSize;

const double _kTopSectionCommunityAvatarSize = 40.0;
const double _kTopSectionProfileAvatarSize = 36.0;
const double _kTopSectionBottomPadding = Grid.xxs;

/// The top section's avatars are 40dp circles, which fill their box edge to
/// edge; the channel rows below lead with an 18dp glyph left-aligned in a 22dp
/// box at [_kChannelSectionInset]. Edge-aligning the two leaves the circles
/// looking pushed outward, so the bar is pulled in to sit the avatar's centre
/// near the channel-icon column.
const double _kTopSectionInset = Grid.twelve;
const Duration _kSectionExpandDuration = Duration(milliseconds: 220);
const Duration _kSectionCollapseDuration = Duration(milliseconds: 170);
const Curve _kSectionExpandCurve = Cubic(0.23, 1, 0.32, 1);
const Curve _kSectionCollapseCurve = Curves.easeInCubic;
const double _kSectionCollapsedScaleY = 0.98;
const double _kHeaderFrostScrollDistance = Grid.xxl;
const double _kHeaderFrostMaxBlurSigma = 23.12;

class _UnreadChannelState {
  final Set<String> ids;

  const _UnreadChannelState({required this.ids});
}

_UnreadChannelState _computeUnreadChannelState({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) {
  if (!readState.isReady) {
    return const _UnreadChannelState(ids: {});
  }

  final latestObservedByChannel = channelsNotifier.latestObservedByChannel;
  final observedEventsByChannel =
      channelsNotifier.observedUnreadEventsByChannel;
  final ids = <String>{};

  for (final channel in channels) {
    if (readState.locallyForcedChannelIds.contains(channel.id)) {
      ids.add(channel.id);
      continue;
    }

    final latestObserved = latestObservedByChannel[channel.id];
    if (latestObserved == null) continue;

    final channelReadAt = readState.effectiveTimestamp(channel.id);
    if (channelReadAt != null && latestObserved <= channelReadAt) continue;

    final observedEvents = observedEventsByChannel[channel.id];
    int? readAtForObservedEvent(ObservedUnreadEvent event) =>
        observedUnreadEventReadAt(
          event,
          channelReadAt,
          (rootId) => readState.effectiveTimestamp(threadContextKey(rootId)),
          (messageId) => readState.effectiveTimestamp(msgContextKey(messageId)),
        );

    final unreadCount = countUnreadObservedEvents(
      observedEvents,
      readAtForObservedEvent,
    );
    if (unreadCount == 0) continue;

    ids.add(channel.id);
  }

  return _UnreadChannelState(ids: ids);
}

class ChannelsPage extends HookConsumerWidget {
  const ChannelsPage({
    required this.settingsPageBuilder,
    this.communityInvitePageBuilder,
    this.communityAppearancePageBuilder,
    required this.onSettingsTransitionProgress,
    this.tabReselection,
    super.key,
  });

  final WidgetBuilder settingsPageBuilder;

  /// Builds the invite destination opened from the community sheet.
  final WidgetBuilder? communityInvitePageBuilder;

  /// Builds the appearance destination opened from the community sheet.
  final WidgetBuilder? communityAppearancePageBuilder;

  /// Reports Settings route progress so its foreground and Home's background
  /// render from the same timeline.
  final ValueChanged<double> onSettingsTransitionProgress;

  /// Notifies this page when its already-selected tab is tapped again.
  final ValueListenable<int>? tabReselection;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Resolve permissions before the community menu is opened.
    if (communityInvitePageBuilder != null) {
      ref.watch(currentCommunityRoleProvider);
    }
    final channelsAsync = ref.watch(channelsProvider);
    final sessionState = ref.watch(relaySessionProvider);
    final currentPubkey = ref
        .watch(profileProvider)
        .whenData((value) => value?.pubkey)
        .value;
    final headerTitleStyle = context.textTheme.titleMedium?.copyWith(
      fontSize: 22,
      fontWeight: FontWeight.w600,
      color: navigationPrimaryForeground(context),
    );
    final topSectionHeight = frostedAppBarHeight(
      context,
      titleStyle: headerTitleStyle,
      bottomHeight: _kTopSectionBottomPadding,
      nativeLargeTitle: true,
    );
    final communityAvatarKey = useMemoized(GlobalKey.new);
    final nativeCommunityAvatarBounds = useRef<Rect?>(null);
    final headerKey = useMemoized(GlobalKey.new);
    final nativeHeaderReady = useRef(false);
    final bodyReady = useRef(false);
    final communityFlightActive = useState(false);
    final channelsScrollController = useScrollController();
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final headerFrostProgress = useState(0.0);
    useEffect(() {
      void updateHeaderTreatment() {
        final nextProgress = !channelsScrollController.hasClients
            ? 0.0
            : (channelsScrollController.offset / _kHeaderFrostScrollDistance)
                  .clamp(0.0, 1.0)
                  .toDouble();
        if ((headerFrostProgress.value - nextProgress).abs() > 0.001) {
          headerFrostProgress.value = nextProgress;
        }
      }

      channelsScrollController.addListener(updateHeaderTreatment);
      return () =>
          channelsScrollController.removeListener(updateHeaderTreatment);
    }, [channelsScrollController]);
    useEffect(() {
      final tabReselection = this.tabReselection;
      if (tabReselection == null) return null;

      void scrollToTop() {
        if (!channelsScrollController.hasClients) return;
        final position = channelsScrollController.position;
        if (position.pixels <= position.minScrollExtent + 0.5) return;
        if (reducedMotion) {
          channelsScrollController.jumpTo(position.minScrollExtent);
          return;
        }
        unawaited(
          channelsScrollController.animateTo(
            position.minScrollExtent,
            duration: const Duration(milliseconds: 260),
            curve: Curves.easeOutCubic,
          ),
        );
      }

      tabReselection.addListener(scrollToTop);
      return () => tabReselection.removeListener(scrollToTop);
    }, [tabReselection, channelsScrollController, reducedMotion]);

    // Cache the last successfully loaded channels so the UI never flashes
    // back to a loading state when the provider rebuilds (e.g. reconnect).
    // Clear the cache on community switch so we show a full loader instead of
    // stale channels from the previous community. unwrapPrevious() ensures the
    // selector sees null during loading (not the previous community's ID).
    final activeCommunityId = ref.watch(
      activeCommunityProvider.select((v) => v.unwrapPrevious().value?.id),
    );
    final cachedChannels = useRef<List<Channel>?>(null);
    final lastCommunityId = useRef<String?>(null);
    if (lastCommunityId.value != activeCommunityId) {
      cachedChannels.value = null;
      lastCommunityId.value = activeCommunityId;
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

    // Only surface fetch errors while the relay is stably connected. During a
    // reconnect the session owns recovery, so a cancelled in-flight query must
    // not turn into a manual Retry page.
    final showError = useState(false);
    final hasError = channelsAsync.hasError && channels == null;
    final canSurfaceError =
        hasError &&
        sessionState.status != SessionStatus.connecting &&
        sessionState.status != SessionStatus.reconnecting;
    useEffect(() {
      if (!canSurfaceError) {
        showError.value = false;
        return null;
      }
      final timer = Timer(const Duration(seconds: 2), () {
        showError.value = true;
      });
      return timer.cancel;
    }, [canSurfaceError]);

    Rect? measureCommunityAvatar() {
      if (defaultTargetPlatform == TargetPlatform.iOS) {
        final header = headerKey.currentContext?.findRenderObject();
        final bounds = nativeCommunityAvatarBounds.value;
        return header is RenderBox && bounds != null
            ? MatrixUtils.transformRect(header.getTransformTo(null), bounds)
            : null;
      }
      final avatar = communityAvatarKey.currentContext?.findRenderObject();
      return avatar is RenderBox
          ? MatrixUtils.transformRect(
              avatar.getTransformTo(null),
              Offset.zero & avatar.size,
            )
          : null;
    }

    Future<Rect?> prepareCommunityLanding() async {
      // The list snapshot precedes unread history and DM profile hydration.
      // Both can still change weight, labels, and sorting behind the picker.
      final channels = ref.read(channelsProvider).requireValue;
      final dmPubkeys = {
        for (final channel in channels)
          if (channel.isMember && !channel.isArchived && channel.isDm)
            ...channel.participantPubkeys,
      };
      await Future.wait([
        ref.read(channelsProvider.notifier).waitForUnreadCatchUp(),
        if (dmPubkeys.isNotEmpty)
          ref.read(userCacheProvider.notifier).preload(dmPubkeys.toList()),
      ]);
      if (!context.mounted || !communityFlightActive.value) return null;
      // Settle Home's scale and scroll while the loading backdrop is opaque.
      onSettingsTransitionProgress(0);
      if (channelsScrollController.hasClients) {
        channelsScrollController.jumpTo(
          channelsScrollController.position.minScrollExtent,
        );
      }
      Rect? previous;
      var stableFrames = 0;
      while (context.mounted && communityFlightActive.value) {
        await WidgetsBinding.instance.endOfFrame;
        if (!context.mounted) return null;
        final ready =
            ref.read(_communityContentReadyProvider).value == true &&
            bodyReady.value &&
            (defaultTargetPlatform != TargetPlatform.iOS ||
                nativeHeaderReady.value);
        final bounds = ready ? measureCommunityAvatar() : null;
        stableFrames = ready && bounds == previous ? stableFrames + 1 : 0;
        previous = bounds;
        if (ready && stableFrames >= 2) return bounds;
      }
      return null;
    }

    void openCommunityGrid() {
      if (!context.mounted) return;
      ref.invalidate(communityIconProvider);
      final destination = measureCommunityAvatar();
      late final _CommunitySwitcherRoute route;
      route = _CommunitySwitcherRoute(
        onTransitionProgress: onSettingsTransitionProgress,
        builder: (_) => _CommunitySwitcherPage(
          destination: destination,
          prepareLanding: prepareCommunityLanding,
          onFlightChanged: (flying) {
            route.flying = flying;
            communityFlightActive.value = flying;
          },
          onTransitionProgress: onSettingsTransitionProgress,
        ),
      );
      Navigator.of(context).push(route).whenComplete(() {
        if (context.mounted) communityFlightActive.value = false;
      });
    }

    void openCommunitySwitcher() {
      // Freeze the menu shape for this presentation. If a cold permission
      // lookup is still pending, the next opening uses its resolved result.
      final role = ref.read(currentCommunityRoleProvider).unwrapPrevious();
      final canInvite = role.hasError || canManageCommunityInvites(role.value);
      unawaited(HapticFeedback.selectionClick());
      showBuzzModalBottomSheet<void>(
        context: context,
        showCloseButton: false,
        showDragHandle: true,
        builder: (_) => _CommunityMenuSheet(
          canInvite: canInvite,
          onSwitchCommunity: openCommunityGrid,
          invitePageBuilder: communityInvitePageBuilder,
          appearancePageBuilder: communityAppearancePageBuilder,
        ),
      );
    }

    final activeCommunity = ref
        .watch(activeCommunityProvider)
        .unwrapPrevious()
        .value;
    final communityRelay = activeCommunity?.relayUrl;
    final communityAvatar = communityRelay == null
        ? null
        : ref.watch(communityIconPresentationProvider(communityRelay));
    final profile = ref.watch(profileProvider).unwrapPrevious().value;
    final communityName = activeCommunity?.name.trim() ?? '';
    final topSectionGradient = context.appColors.topSectionGradient;
    final usesPinnedGradient = topSectionGradient != null;

    return FrostedScaffold(
      backgroundColor: usesPinnedGradient
          ? Colors.transparent
          : context.colors.surface,
      backgroundGradient: topSectionGradient,
      appBar: FrostedAppBar(
        key: headerKey,
        onNativeReadyChanged: (ready) => nativeHeaderReady.value = ready,
        nativeTitle: communityName.isEmpty ? 'Community' : communityName,
        nativeLargeTitle: true,
        nativeLeading: IosNavigationAction(
          label: 'Community settings',
          onAvatarBoundsChanged: (bounds) {
            final header = headerKey.currentContext?.findRenderObject();
            if (header is RenderBox) {
              // Store native coordinates, then apply the current Home transform
              // when measuring the destination immediately before departure.
              nativeCommunityAvatarBounds.value =
                  header.globalToLocal(bounds.topLeft) & bounds.size;
            }
          },
          avatarHidden: communityFlightActive.value,
          avatarIdentity: activeCommunity?.id,
          symbol: 'building.2.crop.circle',
          imageUrl: communityAvatar,
          avatarInitial: communityName.isEmpty
              ? '?'
              : communityName.substring(0, 1).toUpperCase(),
          onPressed: openCommunitySwitcher,
        ),
        nativeActions: [
          IosNavigationAction(
            label: 'Settings',
            avatarIdentity: '${activeCommunity?.id}:${activeCommunity?.pubkey}',
            symbol: 'person.crop.circle',
            imageUrl: profile?.avatarUrl,
            avatarInitial: profile?.initial ?? '?',
            onPressed: () => Navigator.of(context).push(
              _SettingsPageRoute(
                builder: settingsPageBuilder,
                onTransitionProgress: onSettingsTransitionProgress,
              ),
            ),
          ),
        ],
        horizontalInset: _kTopSectionInset,
        // Let the full Buzz gradient show at rest. Once the list begins to
        // move beneath this row, build up blur over the first 64dp of scroll
        // without adding the usual white frosted wash. The Buzz list is
        // transparent, so the blurred pixels remain a continuation of the
        // pinned gradient instead of turning into a white header.
        frosted: !usesPinnedGradient || headerFrostProgress.value > 0,
        frostedSurfaceOpacity: usesPinnedGradient ? 0 : 0.5,
        frostedBlurSigma: usesPinnedGradient
            ? _kHeaderFrostMaxBlurSigma * headerFrostProgress.value
            : 20,
        showBottomDivider: false,
        leading: _CommunityIndicator(
          onTap: openCommunitySwitcher,
          avatarKey: communityAvatarKey,
          hidden: communityFlightActive.value,
        ),
        centerTitle: false,
        titleStyle: headerTitleStyle,
        title: _CommunityHeaderTitle(
          style: headerTitleStyle,
          onTap: openCommunitySwitcher,
        ),
        actions: [
          SizedBox(
            width: Grid.xl,
            height: Grid.xl,
            child: Center(
              child: ProfileAvatar(
                size: _kTopSectionProfileAvatarSize,
                showPresence: false,
                onTap: () {
                  unawaited(HapticFeedback.lightImpact());
                  final route = _SettingsPageRoute(
                    builder: settingsPageBuilder,
                    onTransitionProgress: onSettingsTransitionProgress,
                  );
                  Navigator.of(context).push(route);
                },
              ),
            ),
          ),
        ],
        bottomHeight: _kTopSectionBottomPadding,
        bottom: const SizedBox.expand(),
      ),
      body: _ChannelsBody(
        onReadyChanged: (ready) => bodyReady.value = ready,
        channels: channels,
        channelsAsync: channelsAsync,
        showError: showError.value,
        sessionStatus: sessionState.status,
        currentPubkey: currentPubkey,
        topSectionHeight: topSectionHeight,
        usesPinnedGradient: usesPinnedGradient,
        scrollController: channelsScrollController,
        onRefresh: () => ref.read(channelsProvider.notifier).refresh(),
        onSelectChannel: openChannel,
      ),
    );
  }
}

/// A custom route deliberately avoids [MaterialPageRoute]'s platform exit
/// transition on Home. Settings has a centered scale-and-fade transition, not
/// a lateral page push.
class _SettingsPageRoute extends PageRouteBuilder<void> {
  _SettingsPageRoute({
    required WidgetBuilder builder,
    required this.onTransitionProgress,
  }) : super(
         pageBuilder: (context, animation, secondaryAnimation) =>
             builder(context),
         transitionsBuilder: _buildSettingsTransition,
         opaque: false,
         allowSnapshotting: false,
         transitionDuration: const Duration(milliseconds: 150),
         reverseTransitionDuration: const Duration(milliseconds: 150),
       );

  final ValueChanged<double> onTransitionProgress;

  Animation<double>? _progressAnimation;
  bool _hasStartedForwardTransition = false;

  @override
  void install() {
    super.install();
    _progressAnimation = animation?..addListener(_reportProgress);
  }

  void _reportProgress() {
    final progressAnimation = _progressAnimation;
    if (progressAnimation == null) return;

    // ProxyAnimation briefly exposes the previous completed value while the
    // route installs its new controller. Ignore that handoff notification and
    // begin reporting only once the route is genuinely moving forward.
    if (!_hasStartedForwardTransition) {
      if (progressAnimation.status != AnimationStatus.forward) return;
      _hasStartedForwardTransition = true;
    }
    onTransitionProgress(progressAnimation.value);
  }

  @override
  void dispose() {
    _progressAnimation?.removeListener(_reportProgress);
    super.dispose();
  }

  static Widget _buildSettingsTransition(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) {
    if (MediaQuery.disableAnimationsOf(context)) return child;

    final motion = CurvedAnimation(
      parent: animation,
      // Keep the complete page on one timeline. A gentler forward ease keeps
      // the entrance visible without letting scale finish ahead of opacity;
      // the existing reverse curve preserves the exit motion.
      curve: Curves.easeOutQuad,
      reverseCurve: Curves.easeOutCubic,
    );
    return FadeTransition(
      key: const ValueKey('settings-transition-opacity'),
      opacity: motion,
      child: RepaintBoundary(
        key: const ValueKey('settings-transition-layer'),
        child: ScaleTransition(
          key: const ValueKey('settings-transition-scale'),
          scale: Tween<double>(begin: 1.04, end: 1).animate(motion),
          alignment: Alignment.center,
          child: child,
        ),
      ),
    );
  }
}
