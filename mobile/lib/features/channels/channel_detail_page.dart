import 'dart:async';
import 'dart:math' show min;
import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show ScrollDirection;
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/keyboard_dismiss_on_drag.dart';
import '../../shared/widgets/message_author_meta.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/skeleton.dart';
import '../profile/presence_cache_provider.dart';
import '../profile/profile_provider.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../forum/forum_posts_view.dart';
import 'channel.dart';
import 'channel_actions_sheet.dart';
import 'channel_link_navigation.dart';
import 'agent_activity/working_bots_provider.dart';
import 'channel_management_provider.dart';
import 'channel_sections/channel_sections_provider.dart';
import 'channel_messages_provider.dart';
import 'channel_typing_provider.dart';
import 'channel_typing_indicator.dart';
import 'channels_provider.dart';
import 'unread_badge/observed_unread_event.dart';
import 'compose_bar.dart';
import 'composer_dock_size_reporter.dart';
import 'date_formatters.dart';
import 'day_divider.dart';
import 'dm_channel_labels.dart';
import 'ephemeral_channel_display.dart';
import 'members_sheet.dart';
import 'message_actions.dart';
import 'message_content.dart';
import '../../shared/read_state/deferred_read_state_update.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/read_state/read_state_time.dart';
import 'reaction_row.dart';
import 'send_message_provider.dart';
import '../profile/user_profile_sheet.dart';
import 'small_avatar.dart';
import 'thread_detail_page.dart';
import 'timeline_message.dart';

part 'channel_detail_page/message_list.dart';
part 'channel_detail_page/system_rows.dart';
part 'channel_detail_page/message_bubble.dart';
part 'channel_detail_page/banners.dart';
part 'channel_detail_page/app_bar.dart';

/// Fetch deep-link targets that may be outside the loaded channel window.
Future<void> _loadDeepLinkEvents(
  WidgetRef ref,
  String channelId,
  Set<String> eventIds,
) async {
  try {
    await ref
        .read(channelMessagesProvider(channelId).notifier)
        .loadEventsById(eventIds);
  } catch (error) {
    debugPrint('deep-link: failed to load target messages: $error');
  }
}

/// Fetch channel members and preload their profiles into the user cache.
Future<void> _preloadMembers(WidgetRef ref, String channelId) async {
  // Capture references before async gap to avoid using disposed ref.
  final notifier = ref.read(userCacheProvider.notifier);
  try {
    final members = await ref.read(channelMembersProvider(channelId).future);
    final pubkeys = members.map((m) => m.pubkey).toList();
    if (pubkeys.isNotEmpty) {
      notifier.preload(pubkeys);
    }
  } catch (_) {
    // Non-fatal — mentions will just fall back to cache from messages.
  }
}

int? _channelReadTimestamp({
  required Channel channel,
  required AsyncValue<List<NostrEvent>> messagesState,
}) {
  if (channel.isForum) {
    return dateTimeToUnixSeconds(channel.lastMessageAt);
  }

  final events = messagesState.value;
  if (events != null && events.isNotEmpty) {
    var latest = 0;
    for (final event in events) {
      if (event.threadReference.parentId != null) continue;
      if (event.createdAt > latest) {
        latest = event.createdAt;
      }
    }
    if (latest > 0) {
      return latest;
    }
  }

  return dateTimeToUnixSeconds(channel.lastMessageAt);
}

class ChannelDetailPage extends HookConsumerWidget {
  final Channel channel;
  final String? initialMessageId;
  final String? initialThreadRootId;

  const ChannelDetailPage({
    super.key,
    required this.channel,
    this.initialMessageId,
    this.initialThreadRootId,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final composerDockHeight = useState(0.0);
    final sendMessage = ref.read(sendMessageProvider);
    final detailsAsync = ref.watch(channelDetailsProvider(channel.id));
    final channelsAsync = ref.watch(channelsProvider);
    final messagesState = ref.watch(channelMessagesProvider(channel.id));
    final sessionStatus = ref.watch(relaySessionProvider).status;
    final readState = ref.watch(readStateProvider);
    final channelsNotifier = ref.read(channelsProvider.notifier);
    final initialOrdinaryUnreadMessageIdsRef = useRef<Set<String>>(const {});
    final initialOldestOrdinaryUnreadMessageIdRef = useRef<String?>(null);
    final initialForcedUnreadMessageIdsRef = useRef<Set<String>>(const {});
    final didCaptureInitialReadAt = useRef(false);
    if (readState.isReady && !didCaptureInitialReadAt.value) {
      final channelReadAt = readState.effectiveTimestamp(channel.id);
      final ordinaryUnreadEvents = [
        for (final event
            in channelsNotifier
                    .observedUnreadEventsByChannel[channel.id]
                    ?.values ??
                const <ObservedUnreadEvent>[])
          if (event.rootId == null &&
              event.createdAt >
                  (observedUnreadEventReadAt(
                        event,
                        channelReadAt,
                        (rootId) => readState.effectiveTimestamp(
                          threadContextKey(rootId),
                        ),
                        (messageId) => readState.effectiveTimestamp(
                          msgContextKey(messageId),
                        ),
                      ) ??
                      0))
            event,
      ]..sort((a, b) => a.createdAt.compareTo(b.createdAt));
      initialOrdinaryUnreadMessageIdsRef.value = {
        for (final event in ordinaryUnreadEvents) event.id,
      };
      initialOldestOrdinaryUnreadMessageIdRef.value =
          ordinaryUnreadEvents.firstOrNull?.id;
      initialForcedUnreadMessageIdsRef.value = {
        for (final entry in readState.forcedUnreadContexts.entries)
          if (entry.value == channel.id && entry.key.startsWith('msg:'))
            entry.key.substring('msg:'.length),
      };
      didCaptureInitialReadAt.value = true;
    }
    final initialOrdinaryUnreadMessageIds =
        initialOrdinaryUnreadMessageIdsRef.value;
    final initialOldestOrdinaryUnreadMessageId =
        initialOldestOrdinaryUnreadMessageIdRef.value;
    final initialForcedUnreadMessageIds =
        initialForcedUnreadMessageIdsRef.value;
    final currentPubkey = ref
        .watch(profileProvider)
        .whenData((value) => value?.pubkey)
        .value;
    // Only show channel-level typing (exclude thread-scoped entries and self).
    final typingEntries = ref
        .watch(channelTypingProvider(channel.id))
        .where((e) => e.threadHeadId == null)
        .where(
          (e) =>
              currentPubkey == null ||
              e.pubkey.toLowerCase() != currentPubkey.toLowerCase(),
        )
        .toList();
    final baseChannel =
        channelsAsync
            .whenData(
              (channels) => channels.firstWhere(
                (candidate) => candidate.id == channel.id,
                orElse: () => channel,
              ),
            )
            .value ??
        channel;
    final resolvedChannel =
        detailsAsync.whenData(baseChannel.mergeDetails).value ?? baseChannel;
    final showsComposer =
        !resolvedChannel.isForum &&
        resolvedChannel.isMember &&
        !resolvedChannel.isArchived;
    final messagesNotifier = ref.read(
      channelMessagesProvider(channel.id).notifier,
    );
    final isConnectionInProgress =
        sessionStatus == SessionStatus.connecting ||
        sessionStatus == SessionStatus.reconnecting;
    final showConnectionSkeleton = useState(false);
    final shouldDebounceConnectionSkeleton =
        isConnectionInProgress &&
        (resolvedChannel.isForum || messagesNotifier.hasLoadedMessages);
    useEffect(() {
      if (!shouldDebounceConnectionSkeleton) {
        showConnectionSkeleton.value = false;
        return null;
      }
      final timer = Timer(const Duration(seconds: 2), () {
        showConnectionSkeleton.value = true;
      });
      return timer.cancel;
    }, [shouldDebounceConnectionSkeleton]);
    final showInitialConnectionSkeleton =
        !resolvedChannel.isForum &&
        isConnectionInProgress &&
        !messagesNotifier.hasLoadedMessages;
    final appBarTitleContentHeight = resolvedChannel.isDm
        ? _dmAppBarTitleContentHeight(context)
        : 0.0;
    final readTimestamp = _channelReadTimestamp(
      channel: resolvedChannel,
      messagesState: messagesState,
    );

    useEffect(() {
      final session = ref.read(relaySessionProvider.notifier);
      return session.registerVisibleChannel(channel.id);
    }, [channel.id]);

    // Preload channel member profiles so @mentions resolve correctly.
    useEffect(() {
      _preloadMembers(ref, channel.id);
      return null;
    }, [channel.id]);

    useEffect(
      () {
        if (channel.isForum) return null;
        final eventIds = {
          ?initialMessageId,
          ?initialThreadRootId,
          ?initialOldestOrdinaryUnreadMessageId,
          ...initialForcedUnreadMessageIds,
        };
        if (eventIds.isEmpty) return null;
        final notifier = ref.read(channelMessagesProvider(channel.id).notifier);
        unawaited(_loadDeepLinkEvents(ref, channel.id, eventIds));
        return () => notifier.releaseDeepLinkEvents(eventIds);
      },
      [
        channel.id,
        initialMessageId,
        initialThreadRootId,
        initialOldestOrdinaryUnreadMessageId,
        initialForcedUnreadMessageIds,
      ],
    );

    useEffect(() {
      if (!readState.isReady || readTimestamp == null) {
        return null;
      }
      return deferReadStateUpdate(context, () {
        ref
            .read(readStateProvider.notifier)
            .markContextRead(channel.id, readTimestamp);
        ref
            .read(channelsProvider.notifier)
            .clearObservedUnreadCoveredByRead(channel.id, readTimestamp);
      });
    }, [channel.id, readState.isReady, readTimestamp]);

    return FrostedScaffold(
      appBar: FrostedAppBar(
        iconColor: context.colors.primary,
        titleContentHeight: appBarTitleContentHeight,
        titleStyle: channelTitleTextStyle,
        title: resolvedChannel.isDm
            ? _DmAppBarTitle(
                channel: resolvedChannel,
                currentPubkey: currentPubkey,
              )
            : Row(
                children: [
                  SizedBox.square(
                    dimension: 22,
                    child: Center(
                      child: Icon(channelIcon(resolvedChannel), size: 18),
                    ),
                  ),
                  const SizedBox(width: Grid.half),
                  Expanded(
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Flexible(
                          child: Text(
                            resolveDmChannelDisplayLabel(
                              resolvedChannel,
                              currentPubkey: currentPubkey,
                            ),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                        if (resolvedChannel.isEphemeral) ...[
                          const SizedBox(width: Grid.quarter),
                          _HeaderEphemeralBadge(channel: resolvedChannel),
                        ],
                      ],
                    ),
                  ),
                ],
              ),
        actions: [
          _MembersButton(
            channelId: resolvedChannel.id,
            channel: resolvedChannel,
            currentPubkey: currentPubkey,
          ),
          IconButton(
            color: context.colors.primary,
            onPressed: () async {
              final shouldClose = await showChannelActionsSheet(
                context: context,
                channel: resolvedChannel,
                isUnread: false,
                sectionId: ref
                    .read(channelSectionsProvider)
                    .store
                    .assignments[resolvedChannel.id],
              );
              if (shouldClose == true && context.mounted) {
                Navigator.of(context).pop();
              }
            },
            tooltip: 'Channel actions',
            icon: const Icon(LucideIcons.ellipsisVertical, size: 22),
          ),
        ],
      ),
      body: Stack(
        fit: StackFit.expand,
        children: [
          Column(
            children: [
              Expanded(
                child: resolvedChannel.isForum
                    ? Stack(
                        fit: StackFit.expand,
                        children: [
                          ForumPostsView(
                            channel: resolvedChannel,
                            currentPubkey: currentPubkey,
                          ),
                          if (showConnectionSkeleton.value)
                            Positioned(
                              top:
                                  frostedAppBarHeight(
                                    context,
                                    titleContentHeight:
                                        appBarTitleContentHeight,
                                  ) +
                                  Grid.xs,
                              left: Grid.gutter,
                              right: Grid.gutter,
                              child: _ForumConnectionSkeleton(
                                status: sessionStatus,
                              ),
                            ),
                        ],
                      )
                    : SkeletonReveal(
                        loading:
                            showInitialConnectionSkeleton ||
                            showConnectionSkeleton.value ||
                            messagesState.isLoading,
                        shimmerEnabled:
                            sessionStatus != SessionStatus.disconnected,
                        skeleton: _MessageTimelineSkeleton(
                          appBarTitleContentHeight: appBarTitleContentHeight,
                          status: sessionStatus,
                        ),
                        content: messagesState.when(
                          loading: SizedBox.shrink,
                          error: (e, _) => Padding(
                            padding: EdgeInsets.only(
                              top: frostedAppBarHeight(
                                context,
                                titleContentHeight: appBarTitleContentHeight,
                              ),
                            ),
                            child: Center(
                              child: Text(
                                'Failed to load messages',
                                style: context.textTheme.bodyMedium?.copyWith(
                                  color: context.colors.error,
                                ),
                              ),
                            ),
                          ),
                          data: (events) {
                            final messages = formatTimeline(
                              events,
                              currentPubkey: currentPubkey,
                            );
                            final summaries = ref
                                .read(
                                  channelMessagesProvider(channel.id).notifier,
                                )
                                .threadSummaries;
                            final entries = buildMainTimelineEntries(
                              messages,
                              relaySummaries: summaries,
                            );
                            return _MessageList(
                              entries: entries,
                              allMessages: messages,
                              initialMessageId: initialMessageId,
                              initialThreadRootId: initialThreadRootId,
                              initialOrdinaryUnreadMessageIds:
                                  initialOrdinaryUnreadMessageIds,
                              initialOldestOrdinaryUnreadMessageId:
                                  initialOldestOrdinaryUnreadMessageId,
                              initialForcedUnreadMessageIds:
                                  initialForcedUnreadMessageIds,
                              hasInitialUnread:
                                  readState.isReady &&
                                  (readState.isForcedUnread(channel.id) ||
                                      initialForcedUnreadMessageIds
                                          .isNotEmpty ||
                                      initialOldestOrdinaryUnreadMessageId !=
                                          null),
                              channelId: channel.id,
                              currentPubkey: currentPubkey,
                              isMember: resolvedChannel.isMember,
                              isArchived: resolvedChannel.isArchived,
                              appBarTitleContentHeight:
                                  appBarTitleContentHeight,
                              composerBottomInset: showsComposer
                                  ? composerDockHeight.value
                                  : 0,
                            );
                          },
                        ),
                      ),
              ),
              if (!resolvedChannel.isForum &&
                  (!resolvedChannel.isMember ||
                      resolvedChannel.isArchived)) ...[
                AnimatedSize(
                  duration: MediaQuery.disableAnimationsOf(context)
                      ? Duration.zero
                      : const Duration(milliseconds: 180),
                  curve: Curves.easeOutCubic,
                  alignment: Alignment.bottomCenter,
                  child: typingEntries.isEmpty
                      ? const SizedBox.shrink()
                      : ChannelTypingIndicator(entries: typingEntries),
                ),
                if (!resolvedChannel.isDm)
                  _ReadOnlyNotice(channel: resolvedChannel),
              ],
            ],
          ),
          if (showsComposer)
            Align(
              alignment: Alignment.bottomCenter,
              child: ComposerDockSizeReporter(
                key: const ValueKey('channel-composer-dock'),
                onHeightChanged: (height) {
                  if ((composerDockHeight.value - height).abs() < 0.5) return;
                  composerDockHeight.value = height;
                },
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    AnimatedSize(
                      duration: MediaQuery.disableAnimationsOf(context)
                          ? Duration.zero
                          : const Duration(milliseconds: 180),
                      curve: Curves.easeOutCubic,
                      alignment: Alignment.bottomCenter,
                      child: typingEntries.isEmpty
                          ? const SizedBox.shrink()
                          : ChannelTypingIndicator(entries: typingEntries),
                    ),
                    ComposeBar(
                      channelId: channel.id,
                      channelName: resolvedChannel.isDm
                          ? ''
                          : resolvedChannel.name,
                      onSend:
                          (
                            content,
                            mentionPubkeys, {
                            mediaTags = const <List<String>>[],
                          }) => sendMessage.call(
                            channelId: channel.id,
                            content: content,
                            mentionPubkeys: mentionPubkeys,
                            mediaTags: mediaTags,
                          ),
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
