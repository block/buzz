import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/auth/auth.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../profile/profile_avatar.dart';
import '../profile/profile_provider.dart';
import '../settings/settings_page.dart';
import '../profile/presence_cache_provider.dart';
import '../profile/user_cache_provider.dart';
import '../pairing/pairing_page.dart';
import '../pairing/pairing_provider.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channel_management_provider.dart';
import 'dm_channel_labels.dart';
import 'ephemeral_channel_display.dart';
import 'channel_mutes/channel_mutes_provider.dart';
import 'channel_sections/channel_sections_provider.dart';
import 'channel_sections/channel_sections_storage.dart';
import 'channel_stars/channel_stars_provider.dart';
import 'channels_provider.dart';
import 'read_state/deferred_read_state_update.dart';
import 'read_state/read_state_format.dart';
import 'read_state/read_state_provider.dart';
import 'read_state/read_state_time.dart';
import 'unread_badge/observed_unread_event.dart';

part 'channels_page/body.dart';
part 'channels_page/sections.dart';
part 'channels_page/channel_tile.dart';
part 'channels_page/sheets.dart';
part 'channels_page/badges.dart';
part 'channels_page/community.dart';

enum _QuickAction { createChannel, newDm }

/// Height of the [_ConnectionBanner]: vertical padding (Grid.quarter + 2) × 2
/// plus the ~16px row content (12px spinner / labelSmall text).
const double _kBannerHeight = 24.0;
const double _kChannelSectionInset = Grid.gutter;
const double _kChannelLeadingWidth = 22.0;
const double _kChannelIconSize = 18.0;
const double _kChannelLabelGap = Grid.xxs;
const double _kChannelRowVerticalPadding = Grid.xxs + Grid.quarter;
const double _kChannelLabelInset =
    _kChannelSectionInset + _kChannelLeadingWidth + _kChannelLabelGap;
const double _kCommunityAvatarInset =
    _kChannelSectionInset - Grid.quarter; // FrostedAppBar adds Grid.quarter.
const Duration _kSectionExpandDuration = Duration(milliseconds: 220);
const Duration _kSectionCollapseDuration = Duration(milliseconds: 170);
const Curve _kSectionExpandCurve = Cubic(0.23, 1, 0.32, 1);
const Curve _kSectionCollapseCurve = Curves.easeInCubic;
const double _kSectionCollapsedScaleY = 0.98;

class _UnreadChannelState {
  final Set<String> ids;
  final Map<String, int> counts;

  const _UnreadChannelState({required this.ids, required this.counts});
}

_UnreadChannelState _computeUnreadChannelState({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) {
  if (!readState.isReady) {
    return const _UnreadChannelState(ids: {}, counts: {});
  }

  final latestObservedByChannel = channelsNotifier.latestObservedByChannel;
  final observedEventsByChannel =
      channelsNotifier.observedUnreadEventsByChannel;
  final ids = <String>{};
  final counts = <String, int>{};

  for (final channel in channels) {
    if (readState.locallyForcedChannelIds.contains(channel.id)) {
      ids.add(channel.id);
      counts[channel.id] = 1;
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
    counts[channel.id] = countUnreadBadgeObservedEvents(
      observedEvents,
      readAtForObservedEvent,
    );
  }

  return _UnreadChannelState(ids: ids, counts: counts);
}

class ChannelsPage extends HookConsumerWidget {
  const ChannelsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channelsAsync = ref.watch(channelsProvider);
    final sessionState = ref.watch(relaySessionProvider);
    final currentPubkey = ref
        .watch(profileProvider)
        .whenData((value) => value?.pubkey)
        .value;

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

    Future<void> openQuickActions() async {
      final action = await showModalBottomSheet<_QuickAction>(
        context: context,
        showDragHandle: true,
        builder: (_) => const _QuickActionsSheet(),
      );

      if (!context.mounted || action == null) {
        return;
      }

      switch (action) {
        case _QuickAction.createChannel:
          final created = await showModalBottomSheet<Channel>(
            context: context,
            isScrollControlled: true,
            showDragHandle: true,
            builder: (_) => const _CreateChannelSheet(channelType: 'stream'),
          );
          if (created != null && context.mounted) {
            await openChannel(created);
          }
        case _QuickAction.newDm:
          final opened = await showModalBottomSheet<Channel>(
            context: context,
            isScrollControlled: true,
            showDragHandle: true,
            builder: (_) =>
                _NewDirectMessageSheet(currentPubkey: currentPubkey),
          );
          if (opened != null && context.mounted) {
            await openChannel(opened);
          }
      }
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

    // Match desktop's degraded-state debounce: cached content remains steady
    // through brief socket flaps, and the banner appears only for a sustained
    // reconnect.
    final showConnectionBanner = useState(false);
    final isReconnectingWithContent =
        channels != null &&
        (sessionState.status == SessionStatus.connecting ||
            sessionState.status == SessionStatus.reconnecting);
    useEffect(() {
      if (!isReconnectingWithContent) {
        showConnectionBanner.value = false;
        return null;
      }
      final timer = Timer(const Duration(seconds: 2), () {
        showConnectionBanner.value = true;
      });
      return timer.cancel;
    }, [isReconnectingWithContent]);

    return FrostedScaffold(
      appBar: FrostedAppBar(
        leading: _CommunityIndicator(
          onTap: () => showModalBottomSheet<void>(
            context: context,
            showDragHandle: true,
            builder: (_) => const _CommunitySwitcherSheet(),
          ),
        ),
        title: const SizedBox.shrink(),
        actions: [
          ProfileAvatar(
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const SettingsPage()),
            ),
          ),
          const SizedBox(width: Grid.twelve + Grid.quarter),
        ],
      ),
      floatingActionButton: FloatingActionButton(
        heroTag: 'channels-fab',
        onPressed: openQuickActions,
        tooltip: 'Create or start conversation',
        backgroundColor: context.colors.primary,
        foregroundColor: context.colors.onPrimary,
        shape: const CircleBorder(),
        child: const Icon(LucideIcons.plus),
      ),
      body: _ChannelsBody(
        channels: channels,
        channelsAsync: channelsAsync,
        showError: showError.value,
        sessionStatus: sessionState.status,
        showConnectionBanner: showConnectionBanner.value,
        currentPubkey: currentPubkey,
        onRefresh: () => ref.read(channelsProvider.notifier).refresh(),
        onSelectChannel: openChannel,
      ),
    );
  }
}
