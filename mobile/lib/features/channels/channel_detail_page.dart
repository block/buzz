import 'dart:async';
import 'dart:math' show cos, max, min, pi;
import 'dart:ui' show ImageFilter, lerpDouble;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show ScrollDirection;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

import '../../shared/animated_avatar.dart';
import '../../shared/emoji/emoji_burst.dart';
import '../../shared/huddle/huddle.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/bouncing_dots_indicator.dart';
import '../../shared/widgets/concentric_sheet_surface.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/flapping_bee.dart';
import '../../shared/widgets/keyboard_dismiss_on_drag.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/masked_avatar_badge.dart';
import '../../shared/widgets/message_author_meta.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/skeleton.dart';
import '../profile/presence_cache_provider.dart';
import '../profile/profile_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import '../forum/forum_posts_view.dart';
import 'android_ime_lift.dart';
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
import 'emoji_picker.dart';
import 'ime_metrics_settle_observer.dart';
import 'jump_to_latest_button.dart';
import 'jump_to_latest_switcher.dart';
import 'local_message_send_animation_provider.dart';
import 'local_message_send_transition.dart';
import 'mobile_huddle_controller.dart';
import 'members_sheet.dart';
import 'message_actions.dart';
import 'message_action_backdrop_state.dart';
import 'message_long_press_region.dart';
import 'message_content.dart';
import '../../shared/read_state/deferred_read_state_update.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/read_state/read_state_time.dart';
import 'reaction_row.dart';
import 'recent_emoji_provider.dart';
import 'send_message_provider.dart';
import '../profile/user_profile_sheet.dart';
import 'small_avatar.dart';
import 'sticky_date_header.dart';
import 'thread_detail_page.dart';
import 'timeline_message.dart';

part 'channel_detail_page/message_list.dart';
part 'channel_detail_page/system_rows.dart';
part 'channel_detail_page/huddle_sheet.dart';
part 'channel_detail_page/huddle_call_avatar.dart';
part 'channel_detail_page/huddle_participant_cluster.dart';
part 'channel_detail_page/huddle_call_participants.dart';
part 'channel_detail_page/huddle_participant_overlay.dart';
part 'channel_detail_page/huddle_call_controls.dart';
part 'channel_detail_page/huddle_drawer.dart';
part 'channel_detail_page/huddle_reactions.dart';
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
/// One-to-one DMs additionally refresh participant profiles for identity gates.
/// Returns whether identity resolution completed successfully.
Future<bool> _preloadMembers(
  WidgetRef ref,
  String channelId,
  List<String> participantPubkeys, {
  required bool refreshDmParticipants,
}) async {
  // Capture references before async gap to avoid using disposed ref.
  final notifier = ref.read(userCacheProvider.notifier);
  try {
    final members = await ref.read(channelMembersProvider(channelId).future);
    await notifier.preload(members.map((member) => member.pubkey).toList());
    if (refreshDmParticipants) {
      return notifier.refresh(participantPubkeys);
    }
    return true;
  } catch (_) {
    // Identity remains unresolved, so agent-only actions stay hidden.
    return false;
  }
}

Future<void Function()> _subscribeToDmIdentityUpdates(
  WidgetRef ref,
  List<String> participantPubkeys, {
  required ValueChanged<bool> onReadyChanged,
  required ValueChanged<Set<String>> onAgentPubkeysChanged,
  required VoidCallback onFailure,
}) async {
  final session = ref.read(relaySessionProvider.notifier);
  var subscriptionStatus = RelaySubscriptionStatus.retrying;
  var directLookupComplete = false;
  final agentPubkeys = <String>{};

  void publishAgentPubkeys() {
    onAgentPubkeysChanged(Set.unmodifiable(agentPubkeys));
  }

  void handleEvent(NostrEvent event) {
    if (event.kind == 0) {
      try {
        ref.read(userCacheProvider.notifier).cacheProfileEvent(event);
      } catch (error) {
        debugPrint('[DmIdentity] invalid live profile: $error');
        onFailure();
      }
    } else if (event.kind == 10100) {
      agentPubkeys.add(event.pubkey.toLowerCase());
      publishAgentPubkeys();
      ref.invalidate(agentDirectoryProvider);
      ref.invalidate(agentOwnersProvider);
    }
  }

  final unsubscribe = await session.subscribeWithStatus(
    NostrFilter(
      kinds: const [0, 10100],
      authors: participantPubkeys,
      limit: 100,
    ).copyWithSince(DateTime.now().millisecondsSinceEpoch ~/ 1000 - 5),
    handleEvent,
    onClosed: (_) => onFailure(),
    onStatusChanged: (status) {
      subscriptionStatus = status;
      if (status == RelaySubscriptionStatus.retrying) {
        onReadyChanged(false);
      } else if (directLookupComplete) {
        onReadyChanged(true);
      }
    },
  );

  try {
    final profiles = await session.fetchHistory(
      NostrFilter(
        kinds: const [10100],
        authors: participantPubkeys,
        limit: participantPubkeys.length,
      ),
    );
    for (final profile in profiles) {
      if (profile.kind == 10100) {
        agentPubkeys.add(profile.pubkey.toLowerCase());
      }
    }
    publishAgentPubkeys();
    directLookupComplete = true;
    onReadyChanged(subscriptionStatus == RelaySubscriptionStatus.ready);
    return unsubscribe;
  } catch (_) {
    unsubscribe();
    rethrow;
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

bool _isOneToOneAgentDm(Channel channel, Set<String> agentPubkeys) {
  final participants = channel.participantPubkeys
      .map((pubkey) => pubkey.trim().toLowerCase())
      .where((pubkey) => pubkey.isNotEmpty)
      .toSet();
  return channel.isDm &&
      participants.length == 2 &&
      participants.any(agentPubkeys.contains);
}

/// Controls how a hydrated initial thread is added to the navigation stack.
enum InitialThreadRouteBehavior {
  /// Keep the channel route beneath the thread.
  push,

  /// Replace the temporary channel route so Back returns to its origin.
  replaceCurrentRoute,
}

class ChannelDetailPage extends HookConsumerWidget {
  final Channel channel;
  final String? initialMessageId;
  final String? initialThreadRootId;

  /// How the automatically opened initial thread affects the route stack.
  final InitialThreadRouteBehavior initialThreadRouteBehavior;

  const ChannelDetailPage({
    super.key,
    required this.channel,
    this.initialMessageId,
    this.initialThreadRootId,
    this.initialThreadRouteBehavior = InitialThreadRouteBehavior.push,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final composerDockHeight = useState(0.0);
    final composerFocusNode = useFocusNode();
    final restoreComposerFocus = useRef<VoidCallback?>(null);
    final sendMessage = ref.read(sendMessageProvider);
    final detailsAsync = ref.watch(channelDetailsProvider(channel.id));
    final channelsAsync = ref.watch(channelsProvider);
    final messagesState = ref.watch(channelMessagesProvider(channel.id));
    final huddleLifecycle =
        ref.watch(huddleLifecycleProvider(channel.id)).value ?? const [];
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
    final participantCount = resolvedChannel.participantPubkeys
        .map((pubkey) => pubkey.trim().toLowerCase())
        .where((pubkey) => pubkey.isNotEmpty)
        .toSet()
        .length;
    final isOneToOneDm = resolvedChannel.isDm && participantCount == 2;
    final memberProfilesPreload = useMemoized(
      () => _preloadMembers(
        ref,
        resolvedChannel.id,
        resolvedChannel.participantPubkeys,
        refreshDmParticipants: isOneToOneDm,
      ),
      [
        resolvedChannel.id,
        sessionStatus,
        isOneToOneDm,
        Object.hashAll(resolvedChannel.participantPubkeys),
      ],
    );
    final memberProfilesPreloadState = useFuture(memberProfilesPreload);
    final showsComposer =
        !resolvedChannel.isForum &&
        resolvedChannel.isMember &&
        !resolvedChannel.isArchived;
    final profileOwnedAgentPubkeys = <String>[];
    for (final participantPubkey in resolvedChannel.participantPubkeys) {
      final normalized = participantPubkey.trim().toLowerCase();
      final isProfileOwnedAgent = ref.watch(
        userCacheProvider.select(
          (cache) => cache[normalized]?.ownerPubkey != null,
        ),
      );
      if (isProfileOwnedAgent) profileOwnedAgentPubkeys.add(normalized);
    }
    final agentDirectoryState = ref.watch(agentDirectoryProvider);
    final agentOwnersState = ref.watch(agentOwnersProvider);
    final channelMembershipUpdateState = isOneToOneDm
        ? ref.watch(channelMembershipUpdateProvider(resolvedChannel.id))
        : const ChannelMembershipUpdateState(isReady: true);
    final channelBotPubkeysState = ref.watch(
      channelBotPubkeysProvider(resolvedChannel.id),
    );
    final identitySubscriptionPubkeys = isOneToOneDm
        ? (resolvedChannel.participantPubkeys
              .map((pubkey) => pubkey.trim().toLowerCase())
              .where((pubkey) => pubkey.isNotEmpty)
              .toSet()
              .toList()
            ..sort())
        : const <String>[];
    final identitySubscriptionKey = Object.hashAll(identitySubscriptionPubkeys);
    final identitySubscriptionReady = useValueNotifier(false, [
      sessionStatus,
      resolvedChannel.id,
      identitySubscriptionKey,
    ]);
    final directlyResolvedAgentPubkeys = useValueNotifier(<String>{}, [
      sessionStatus,
      resolvedChannel.id,
      identitySubscriptionKey,
    ]);
    final isIdentitySubscriptionReady = useValueListenable(
      identitySubscriptionReady,
    );
    final directAgentPubkeys = useValueListenable(directlyResolvedAgentPubkeys);
    final agentPubkeys = agentPubkeysWithChannelBots(
      knownAgentPubkeys: agentPubkeysWithProfileOwners(
        knownAgentPubkeys: {
          ...ref.watch(knownAgentPubkeysProvider),
          ...directAgentPubkeys,
        },
        profileOwnedAgentPubkeys: profileOwnedAgentPubkeys,
      ),
      channelBotPubkeys:
          channelBotPubkeysState.asData?.value ?? const <String>{},
    );
    useEffect(() {
      if (sessionStatus != SessionStatus.connected ||
          identitySubscriptionPubkeys.isEmpty) {
        return null;
      }
      var disposed = false;
      var subscriptionFailed = false;
      void markFailed() {
        subscriptionFailed = true;
        if (!disposed) identitySubscriptionReady.value = false;
      }

      void Function()? unsubscribe;
      Future.microtask(() async {
        try {
          final cleanup = await _subscribeToDmIdentityUpdates(
            ref,
            identitySubscriptionPubkeys,
            onReadyChanged: (isReady) {
              if (!disposed && !subscriptionFailed) {
                identitySubscriptionReady.value = isReady;
              }
            },
            onAgentPubkeysChanged: (pubkeys) {
              if (!disposed) directlyResolvedAgentPubkeys.value = pubkeys;
            },
            onFailure: markFailed,
          );
          if (disposed) {
            cleanup();
          } else {
            unsubscribe = cleanup;
          }
        } catch (error) {
          if (!disposed) {
            debugPrint('[DmIdentity] live subscription failed: $error');
            markFailed();
          }
        }
      });
      return () {
        disposed = true;
        unsubscribe?.call();
      };
    }, [sessionStatus, resolvedChannel.id, identitySubscriptionKey]);
    final isAgentIdentityUnresolved =
        isOneToOneDm &&
        (sessionStatus != SessionStatus.connected ||
            !isIdentitySubscriptionReady ||
            agentDirectoryState.isLoading ||
            agentDirectoryState.hasError ||
            agentOwnersState.isLoading ||
            agentOwnersState.hasError ||
            !channelMembershipUpdateState.isReady ||
            channelMembershipUpdateState.error != null ||
            channelBotPubkeysState.isLoading ||
            channelBotPubkeysState.hasError ||
            memberProfilesPreloadState.connectionState !=
                ConnectionState.done ||
            memberProfilesPreloadState.data != true);
    final showsHuddleAction =
        showsComposer &&
        !isAgentIdentityUnresolved &&
        !_isOneToOneAgentDm(resolvedChannel, agentPubkeys);
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
    final appBarTitleContentHeight = _twoLineAppBarTitleContentHeight(
      context,
      isDm: resolvedChannel.isDm,
    );
    final usesNativeIosGlassBackButton =
        Navigator.canPop(context) &&
        Theme.of(context).platform == TargetPlatform.iOS;
    final readTimestamp = _channelReadTimestamp(
      channel: resolvedChannel,
      messagesState: messagesState,
    );

    useEffect(() {
      final session = ref.read(relaySessionProvider.notifier);
      return session.registerVisibleChannel(channel.id);
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
      resizeToAvoidBottomInset:
          !usesFixedAndroidImeViewport || resolvedChannel.isForum,
      appBar: FrostedAppBar(
        leading: usesNativeIosGlassBackButton
            ? IosGlassNavigationButton(
                key: const ValueKey('channel-ios-glass-back'),
                icon: IosGlassNavigationIcon.back,
                semanticLabel: 'Back',
                onPressed: () => Navigator.of(context).maybePop(),
                width: iosGlassChannelHeaderLeadingWidth,
                buttonCenterX: iosGlassChannelHeaderButtonCenterX,
                nativeViewSuppressed: messageActionBackdropActive,
              )
            : null,
        iconColor: context.colors.primary,
        titleContentHeight: appBarTitleContentHeight,
        titleStyle: channelTitleTextStyle,
        title: Padding(
          padding: EdgeInsets.only(
            left: usesNativeIosGlassBackButton
                ? iosGlassChannelHeaderTitleSpacing
                : 0,
          ),
          child: resolvedChannel.isDm
              ? _DmAppBarTitle(
                  channel: resolvedChannel,
                  currentPubkey: currentPubkey,
                )
              : _ChannelAppBarTitle(
                  channel: resolvedChannel,
                  onTap: () async {
                    final shouldClose = await showChannelDetailsPage(
                      context: context,
                      channel: resolvedChannel,
                      currentPubkey: currentPubkey,
                      onMemberTap: showUserProfileSheet,
                      sectionId: ref
                          .read(channelSectionsProvider)
                          .store
                          .assignments[resolvedChannel.id],
                    );
                    if (shouldClose == true && context.mounted) {
                      Navigator.of(context).pop();
                    }
                  },
                ),
        ),
        actions: resolvedChannel.isDm
            ? [
                if (showsHuddleAction)
                  _HuddleButton(
                    channel: resolvedChannel,
                    events: [
                      ...messagesState.value ?? const [],
                      ...huddleLifecycle,
                    ],
                  ),
                if (_showsMembersAction(resolvedChannel))
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
              ]
            : [
                if (showsComposer)
                  _HuddleButton(
                    channel: resolvedChannel,
                    events: [
                      ...messagesState.value ?? const [],
                      ...huddleLifecycle,
                    ],
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
                              initialThreadRouteBehavior:
                                  initialThreadRouteBehavior,
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
                              composerFocusNode: showsComposer
                                  ? composerFocusNode
                                  : null,
                              restoreComposerFocus: showsComposer
                                  ? () => restoreComposerFocus.value?.call()
                                  : null,
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
            AndroidImeLift(
              child: Align(
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
                        focusNode: composerFocusNode,
                        onFocusRestorerChanged: (restoreFocus) =>
                            restoreComposerFocus.value = restoreFocus,
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
                              channel: resolvedChannel,
                              mediaTags: mediaTags,
                            ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
