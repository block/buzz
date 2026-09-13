import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/push/push_presentation_cache.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme_provider.dart';
import '../../shared/utils/string_utils.dart';
import 'channel.dart';
import 'channel_management_provider.dart'
    show ChannelMember, channelDetailsProvider;
import 'channel_mutes/channel_mutes_provider.dart';
import 'huddle_channel_filter.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'thread_follows/thread_follows_provider.dart';
import 'unread_badge/is_high_priority_event.dart';
import 'unread_badge/observed_unread_event.dart';
import 'unread_badge/should_notify_for_event.dart';

part 'channel_directory.dart';
part 'channel_member_snapshots.dart';
part 'channels_provider_lifecycle.dart';

const _channelTypeOrder = {'stream': 0, 'forum': 1, 'dm': 2};
const _unreadCatchUpLimit = 1000;
const _participatedRootIdsPrefix = 'buzz-thread-participation.v1';
const _authoredRootIdsPrefix = 'buzz-thread-authored.v1';

/// Loads the user's channel list from the relay over WebSocket.
///
/// Membership loading resolves kind:39002 events tagged `#p:<my-pubkey>`,
/// then fetches kind:39000 metadata for those channel ids.
///
/// The paginated kind:39000 directory is fetched separately when Browse
/// channels opens, so discovery never delays the main Conversations screen.
/// Live updates are layered on top via chunked subscriptions on the `#h` tag
/// for any visible channel event kind. Chunks stay within the relay's explicit
/// channel cap and incoming events bump `lastMessageAt` for their channel.
class ChannelsNotifier extends AsyncNotifier<List<Channel>> {
  static const _backstopInterval = Duration(seconds: 60);

  final Map<String, _LiveChunkSubscription> _liveSubscriptionsByChunk = {};
  Future<void> _liveSubscriptionQueue = Future.value();
  Set<String> _desiredLiveChannelIds = const {};
  int _subscriptionVersion = 0;
  int _nextLiveChunkGeneration = 0;
  final Set<String> _terminallyClosedLiveChunks = {};
  String? _subscriptionRelayBaseUrl;
  Timer? _backstopTimer;
  final Map<String, int> _latestObservedByChannel = {};
  final Map<String, Map<String, ObservedUnreadEvent>>
  _observedUnreadEventsByChannel = {};
  Set<String> _participatedRootIds = {};
  Set<String> _authoredRootIds = {};
  String? _threadInterestPubkey;
  bool _hasLoaded = false;
  String? _memberSnapshotRelayBaseUrl;
  String? _memberSnapshotPubkey;
  Map<String, List<ChannelMember>> _memberSnapshotsByChannelId = const {};
  List<NostrEvent> _directoryMetas = const [];
  Set<String> _hiddenDmIds = const {};

  /// Fences directory responses to the relay and identity that requested them.
  late final _ChannelRefreshCoordinator _refreshCoordinator =
      _ChannelRefreshCoordinator.forRef(ref);

  // Expose the notifier's protected Ref only to same-library extensions used to
  // keep lifecycle code below the file-size ratchet.
  Ref get _lifecycleRef => ref;

  /// The member snapshot already returned while loading the channel list.
  ///
  /// Mention autocomplete can use this synchronously while its independent
  /// channel-member refresh is still in flight.
  List<ChannelMember> cachedMembersForChannel(String channelId) =>
      _memberSnapshotsByChannelId[channelId] ?? const [];

  Map<String, int> get latestObservedByChannel =>
      Map.unmodifiable(_latestObservedByChannel);

  Set<String> get hiddenDmIds => Set.unmodifiable(_hiddenDmIds);

  bool get hasLoaded => _hasLoaded;

  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel =>
      Map<String, Map<String, ObservedUnreadEvent>>.unmodifiable({
        for (final entry in _observedUnreadEventsByChannel.entries)
          entry.key: Map<String, ObservedUnreadEvent>.unmodifiable(entry.value),
      });

  @override
  Future<List<Channel>> build() async {
    final relayBaseUrl = ref.watch(relayConfigProvider).baseUrl;
    final pubkey = ref.watch(myPubkeyProvider)?.toLowerCase();
    if (_memberSnapshotRelayBaseUrl != relayBaseUrl ||
        _memberSnapshotPubkey != pubkey) {
      _memberSnapshotRelayBaseUrl = relayBaseUrl;
      _memberSnapshotPubkey = pubkey;
      _memberSnapshotsByChannelId = const {};
      _directoryMetas = const [];
      _hiddenDmIds = const {};
      // Retire any in-flight directory request: its response describes the
      // previous relay or identity and must not reach this scope's state.
      _refreshCoordinator.retireInFlight();
    }
    final connected = Completer<void>();
    final sessionState = ref.read(relaySessionProvider);
    final waitingForInitialConnection =
        sessionState.status != SessionStatus.connected;
    ref.listen(relaySessionProvider, (previous, next) {
      if (next.status != SessionStatus.connected) return;
      if (waitingForInitialConnection &&
          !_hasLoaded &&
          !connected.isCompleted) {
        connected.complete();
      } else if (previous?.status != SessionStatus.connected) {
        // A new authenticated connection can change relay admission policy.
        // Ordinary refreshes must not retry an unchanged terminal rejection.
        _terminallyClosedLiveChunks.clear();
        unawaited(_backstopRefresh());
      }
    });

    // Re-fetch when the app returns to foreground so channels created on
    // another device while mobile was backgrounded appear immediately.
    ref.listen(appLifecycleProvider, (prev, next) {
      if (next == AppLifecycleState.resumed) {
        refresh();
      }
    });

    ref.onDispose(() {
      _clearLiveSubscriptions();
      _latestObservedByChannel.clear();
      _observedUnreadEventsByChannel.clear();
      _backstopTimer?.cancel();
      _backstopTimer = null;
    });

    if (sessionState.status != SessionStatus.connected) {
      // Keep the prior community's cache visible until the new relay connects.
      if (_hasLoaded) return state.value ?? const [];
      await connected.future;
    }

    return _fetch(subscribeLive: true);
  }

  Future<List<Channel>> _fetch({
    bool subscribeLive = false,
    bool fetchLastMessage = true,
    bool fetchDirectory = false,
  }) async {
    final channels = await _fetchChannels(
      subscribeLive: subscribeLive,
      fetchLastMessage: fetchLastMessage,
      fetchDirectory: fetchDirectory,
    );
    _hasLoaded = true;
    return channels;
  }

  Future<List<Channel>> _fetchChannels({
    bool subscribeLive = false,
    bool fetchLastMessage = true,
    bool fetchDirectory = false,
  }) async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) throw StateError('No signing identity available');
    final communityID = ref.read(activeCommunityProvider).value?.id;
    _loadThreadInterestStores(myPk);

    final session = ref.read(relaySessionProvider.notifier);

    // Acquire request ownership before the first relay await. Every channel-list
    // path uses this fence so completion order cannot let an older ordinary,
    // directory, or reconnect refresh replace a newer membership list.
    final fence = _refreshCoordinator.beginRefresh(
      fetchesDirectory: fetchDirectory,
    );

    // Step 1: find the channels I'm a member of via kind:39002.
    final memberships = await _fenced(
      fence,
      _fetchChannelMemberships(session, myPk),
    );
    final memberChannelIds = memberships
        .map((e) => e.getTagValue('d'))
        .whereType<String>()
        .toSet();
    _cacheMemberSnapshots(memberships, replaceAll: true);

    // Step 2: pull metadata for joined channels. A user with no memberships
    // must still continue to directory discovery below.
    final memberMetas = memberChannelIds.isEmpty
        ? const <NostrEvent>[]
        : await _fenced(
            fence,
            session.fetchHistory(
              NostrFilters.channelMetadata(memberChannelIds.toList()),
            ),
          );

    // Step 3: fetch the open-channel directory. The relay filters this global
    // kind:39000 query by the caller's access, but the client still rejects
    // private channels and DMs below so discovery fails closed if that contract
    // ever regresses. The composite cursor preserves tied-timestamp rows.
    if (fetchDirectory) {
      final metas = await _refreshCoordinator.loadDirectory(session, fence);
      if (metas != null) _directoryMetas = metas;
    }

    // Merge and dedupe by `d` tag. Kind:39000 is parameterized-replaceable,
    // but stale revisions from before the relay's d_tag backfill can linger.
    final latestMetaPerId = <String, NostrEvent>{};
    for (final event in [...memberMetas, ..._directoryMetas]) {
      if (event.kind != 39000) continue;
      final id = event.getTagValue('d');
      if (id == null) continue;
      final existing = latestMetaPerId[id];
      if (existing == null ||
          event.createdAt > existing.createdAt ||
          (event.createdAt == existing.createdAt &&
              event.id.compareTo(existing.id) < 0)) {
        latestMetaPerId[id] = event;
      }
    }
    final dedupedMetas = latestMetaPerId.values.toList();

    // Resolve DM participant display names. Extracted into the part file so
    // `channels_provider.dart` stays under the 1200-line ceiling enforced by
    // `just file-size-check`.
    final displayNames = await _resolveDmDisplayNames(
      session,
      fence,
      dedupedMetas,
      myPk,
    );

    final hiddenDmIds = await _fenced(fence, _fetchHiddenDmIds(session, myPk));
    _hiddenDmIds = Set.unmodifiable(hiddenDmIds);
    // Fetch the authoritative membership snapshots before filtering Huddle
    // backing channels. The relay-signed kind:39000 metadata identifies the
    // relay, not the channel creator; the owner role in kind:39002 is the
    // canonical creator identity used to reject forged Huddle links.
    final memberCountChannelIds = memberChannelIds.toList();
    final memberEvents = memberCountChannelIds.isEmpty
        ? const <NostrEvent>[]
        : await _fenced(
            fence,
            session.fetchHistory(
              NostrFilter(
                kinds: const [39002],
                tags: {'#d': memberCountChannelIds},
                limit: memberCountChannelIds.length,
              ),
            ),
          );
    final huddleStarts = memberCountChannelIds.isEmpty
        ? const <NostrEvent>[]
        : await _fenced(
            fence,
            _fetchHuddleStarts(session, memberCountChannelIds),
          );
    final huddleBackingIds = huddleBackingChannelIds(
      huddleStarts,
      memberEvents,
    );

    final channels = <Channel>[];
    for (final event in dedupedMetas) {
      final id = event.getTagValue('d');
      if (id == null) continue;
      final isMember = memberChannelIds.contains(id);
      final channel = _channelFromMeta(
        event,
        isMember: isMember,
        displayNames: displayNames,
      );
      if (!isMember && (channel.isPrivate || channel.isDm)) continue;
      if (channel.isDm && hiddenDmIds.contains(channel.id)) continue;
      if (huddleBackingIds.contains(channel.id) &&
          channel.isStream &&
          channel.isPrivate) {
        continue;
      }
      // Ephemeral (TTL) channels are surfaced in the list with an
      // `_EphemeralBadge` rendered in `channels_page.dart` — they shouldn't be
      // hidden. Desktop shows them too. Previously dropped here unconditionally,
      // which made TTL channels invisible on iOS even when the user was a member.
      channels.add(channel);
    }

    // Use the membership snapshots already fetched above for both Huddle
    // linkage validation and member-count hydration.
    if (memberEvents.isNotEmpty) _cacheMemberSnapshots(memberEvents);
    unawaited(
      cacheBuzzPushChannelEvents(communityID, dedupedMetas, [
        ...memberships,
        ...memberEvents,
      ]),
    );
    final memberCounts = _memberCountsByChannelId(memberEvents);
    for (var i = 0; i < channels.length; i++) {
      final count = memberCounts[channels[i].id];
      if (count != null) {
        channels[i] = channels[i].copyWith(memberCount: count);
      }
    }

    // Step 3: fetch the most recent message per channel to populate lastMessageAt.
    // kind:39000 metadata doesn't carry message timestamps, so channels load with
    // lastMessageAt: null. Without this, unread detection and badge computation
    // see every channel as having no messages. Skipped on backstop refreshes since
    // live subscriptions keep lastMessageAt current after the initial load.
    if (fetchLastMessage) {
      final activeChannels = [
        for (final channel in channels)
          if (channel.isMember && !channel.isArchived) channel,
      ];
      final channelById = {
        for (final channel in activeChannels) channel.id: channel,
      };
      final events = await _fenced(
        fence,
        _fetchLastMessageEvents(session, activeChannels),
      );
      final lastMessageMap = <String, int>{};
      final mutedChannelIds = _mutedChannelIds();
      for (final event in events) {
        final channelId = event.channelId;
        if (channelId == null) continue;
        final channel = channelById[channelId];
        if (channel == null) continue;
        if (!channel.isDm &&
            !shouldNotifyForEvent(
              event,
              myPk,
              mutedChannelIds: mutedChannelIds,
              channelId: channelId,
            )) {
          continue;
        }
        final current = lastMessageMap[channelId];
        if (current == null || event.createdAt > current) {
          lastMessageMap[channelId] = event.createdAt;
        }
      }

      for (var i = 0; i < channels.length; i++) {
        final ts = lastMessageMap[channels[i].id];
        if (ts != null) {
          channels[i] = channels[i].copyWith(
            lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
              ts * 1000,
              isUtc: true,
            ),
          );
        }
      }
    }

    channels.sort((left, right) {
      final typeOrder =
          (_channelTypeOrder[left.channelType] ?? 99) -
          (_channelTypeOrder[right.channelType] ?? 99);
      if (typeOrder != 0) return typeOrder;
      // Case-insensitive to match desktop's `localeCompare` ordering.
      return left.name.toLowerCase().compareTo(right.name.toLowerCase());
    });

    // Invalidate `channelDetailsProvider` entries whose archived state flipped
    // since the last fetch. Required because `channelDetailsProvider` is a
    // separate Riverpod cache and `Channel.mergeDetails(details)` overwrites
    // archivedAt from the cached details — so an active-then-archived channel
    // (e.g. TTL auto-archive by the relay reaper) could keep showing compose
    // and manage actions in the detail view until the cache expired naturally.
    //
    // Scoped narrowly to the archived flip — broader metadata staleness
    // (renames, topic changes, etc.) is a separate, pre-existing concern that
    // already affects this provider for other reasons.
    // Re-check before the first write that other providers can observe. Every
    // await above is fenced, but the switch can also land in the synchronous
    // gap, so the guard sits immediately before the write rather than only
    // after the await.
    fence.ensureCurrent();

    final prevById = <String, Channel>{
      for (final c in state.value ?? const <Channel>[]) c.id: c,
    };
    for (final channel in channels) {
      final prev = prevById[channel.id];
      if (prev != null && prev.isArchived != channel.isArchived) {
        ref.invalidate(channelDetailsProvider(channel.id));
      }
    }

    // Layer live delivery onto the already-built snapshot without making EOSE
    // part of the provider's readiness path. Every refresh queues a generation;
    // a later refresh or scope switch retires older queued/in-flight work.
    if (subscribeLive) {
      unawaited(_subscribeLive(channels, fence));
    }
    // Guard the provider-state write in `retryDirectory` and `build`: the
    // caller assigns whatever this returns, so the last check belongs here.
    fence.ensureCurrent();
    return channels;
  }

  /// Fetches each channel's independent latest-message window in one HTTP
  /// bridge request. The relay preserves NIP-01 per-filter limits while
  /// executing the filters with bounded concurrency, avoiding an unbounded
  /// burst of websocket REQs on communities with many channels.
  Future<List<NostrEvent>> _fetchLastMessageEvents(
    RelaySessionNotifier session,
    List<Channel> channels,
  ) async {
    if (channels.isEmpty) return const [];

    final filters = [
      for (final channel in channels)
        NostrFilter(
          kinds: EventKind.channelMessageEventKinds,
          tags: {
            '#h': [channel.id],
          },
          limit: channel.isDm ? 1 : 20,
        ),
    ];

    return _fetchChannelHistoryBatch(
      session,
      filters,
      operation: 'latest-message query',
    );
  }

  Future<List<NostrEvent>> _fetchChannelHistoryBatch(
    RelaySessionNotifier session,
    List<NostrFilter> filters, {
    required String operation,
  }) async {
    if (filters.isEmpty) return const [];

    try {
      return await session.queryRelay(filters);
    } catch (error) {
      debugPrint(
        '[ChannelsNotifier] batched $operation failed; '
        'using bounded websocket fallback: $error',
      );
    }

    const fallbackConcurrency = 4;
    final events = <NostrEvent>[];
    for (var start = 0; start < filters.length; start += fallbackConcurrency) {
      final end = min(start + fallbackConcurrency, filters.length);
      final results = await Future.wait(
        filters.sublist(start, end).map((filter) async {
          try {
            return await session.fetchHistory(filter);
          } catch (_) {
            return const <NostrEvent>[];
          }
        }),
      );
      for (final result in results) {
        events.addAll(result);
      }
    }
    return events;
  }

  /// Build a [Channel] from a kind:39000 metadata event.
  ///
  /// [displayNames] maps lowercase participant pubkey → resolved label and is
  /// used to populate [Channel.participants] for DMs so [Channel.displayLabel]
  /// can render real names instead of the relay-canonical "DM" name.
  Channel _channelFromMeta(
    NostrEvent event, {
    required bool isMember,
    Map<String, String> displayNames = const {},
  }) {
    final data = ChannelData.fromEvent(event);
    final participants = data.channelType == 'dm'
        ? [
            for (final pk in data.participantPubkeys)
              displayNames[pk.toLowerCase()] ?? shortPubkey(pk),
          ]
        : const <String>[];
    return Channel(
      id: data.id,
      name: data.name,
      channelType: data.channelType,
      visibility: data.visibility,
      description: data.description,
      topic: data.topic,
      createdBy: event.pubkey,
      createdAt: DateTime.fromMillisecondsSinceEpoch(
        event.createdAt * 1000,
        isUtc: true,
      ),
      memberCount: 0,
      lastMessageAt: null,
      // `archivedAt` doubles as both the archived-state flag and the timestamp.
      // The kind:39000 metadata only carries `["archived", "true"]`, not the
      // moment of archival, so we stamp the event's `createdAt` — that's when
      // the relay republished the metadata, which is the closest signal we have.
      archivedAt: data.isArchived
          ? DateTime.fromMillisecondsSinceEpoch(
              event.createdAt * 1000,
              isUtc: true,
            )
          : null,
      participants: participants,
      participantPubkeys: data.participantPubkeys,
      isMember: isMember,
      ttlSeconds: data.ttlSeconds,
      ttlDeadline: data.ttlDeadline,
    );
  }

  /// Backfills unread badges for the channels this refresh just installed.
  ///
  /// Runs detached from the refresh that starts it, so the lifecycle token
  /// captured below is what keeps a response that outlived its refresh from
  /// writing unread state into whatever the user is looking at now. Every
  /// refresh path starts a catch-up, including the initial load, the ordinary
  /// membership refresh a join performs and the reconnect backstop, so the
  /// token is unconditional rather than tied to discovery. A retired refresh
  /// returns instead of throwing: nothing awaits this future, so a thrown
  /// [_StaleChannelRefresh] would only surface as an unhandled error.
  ///
  /// The request fence and subscription generation are passed from the refresh
  /// that installed [channels], preserving request ownership after detachment.
  Future<void> _catchUpUnreadEvents(
    List<Channel> channels,
    _ChannelRefreshFence fence,
    int subscriptionGeneration,
  ) async {
    if (!ref.mounted) return;
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) return;

    final session = ref.read(relaySessionProvider.notifier);
    final mutedChannelIds = _mutedChannelIds();
    final ReadStateState readState;
    try {
      readState = ref.read(readStateProvider);
    } catch (error) {
      debugPrint('[ChannelsNotifier] unread catch-up skipped: $error');
      return;
    }
    final activeChannels = [
      for (final channel in channels)
        if (channel.isMember && !channel.isArchived) channel,
    ];
    final channelById = {
      for (final channel in activeChannels) channel.id: channel,
    };
    final readAtByChannel = {
      for (final channel in activeChannels)
        channel.id: readState.effectiveTimestamp(channel.id),
    };
    final filters = [
      for (final channel in activeChannels)
        NostrFilter(
          kinds: EventKind.channelMessageEventKinds,
          tags: {
            '#h': [channel.id],
          },
          since: (readAtByChannel[channel.id] ?? -1) + 1,
          limit: _unreadCatchUpLimit,
        ),
    ];

    try {
      final events = await _fetchChannelHistoryBatch(
        session,
        filters,
        operation: 'unread catch-up',
      );
      // The relay round-trip above is the window Jed's probes park in: a newer
      // refresh, a community switch or an identity switch here means every
      // write below belongs to a channel list the user has left.
      if (!ref.mounted || _isCatchUpRetired(fence, subscriptionGeneration)) {
        return;
      }

      for (final event in events) {
        if (event.pubkey.toLowerCase() == myPk.toLowerCase()) {
          _recordSelfThreadInterest(event, myPk);
        }
      }

      var recorded = false;
      for (final event in events) {
        final channelId = event.channelId;
        if (channelId == null) continue;
        final channel = channelById[channelId];
        if (channel == null) continue;
        final readAt = readAtByChannel[channelId];
        if (event.pubkey.toLowerCase() == myPk.toLowerCase()) continue;
        if (readAt != null && event.createdAt <= readAt) continue;
        if (!shouldNotifyForEvent(
          event,
          myPk,
          participatedRootIds: _participatedRootIds,
          followedRootIds: _followedRootIds(),
          authoredRootIds: _authoredRootIds,
          mutedChannelIds: mutedChannelIds,
          channelId: channel.id,
        )) {
          continue;
        }
        _recordUnreadEvent(channel, event, myPk);
        recorded = true;
      }
      // Republish only when this catch-up actually changed unread state. A
      // batch that recorded nothing has nothing to show, and a failed or
      // superseded batch must not repaint another refresh's list: the retired
      // check above already returned in that case, and no await separates it
      // from here, so a second check would be dead code.
      if (recorded) {
        state = state.whenData((channels) => List<Channel>.of(channels));
      }
    } catch (error) {
      if (!ref.mounted) return;
      debugPrint('[ChannelsNotifier] unread catch-up failed: $error');
    }
  }

  void _handleLiveEvent(NostrEvent event) {
    final channelId = event.channelId;
    if (channelId == null) return;

    final myPk = ref.read(myPubkeyProvider);
    final mutedChannelIds = _mutedChannelIds();

    state = state.whenData((channels) {
      final idx = channels.indexWhere((c) => c.id == channelId);
      if (idx == -1) {
        refresh();
        return channels;
      }
      final updated = List<Channel>.of(channels);
      final channel = updated[idx];

      if (myPk != null && event.pubkey.toLowerCase() == myPk.toLowerCase()) {
        _recordSelfThreadInterest(event, myPk);
      }

      if (myPk != null &&
          shouldNotifyForEvent(
            event,
            myPk,
            participatedRootIds: _participatedRootIds,
            followedRootIds: _followedRootIds(),
            authoredRootIds: _authoredRootIds,
            mutedChannelIds: mutedChannelIds,
            channelId: channel.id,
          )) {
        _recordUnreadEvent(channel, event, myPk);
        final eventTime = DateTime.fromMillisecondsSinceEpoch(
          event.createdAt * 1000,
          isUtc: true,
        );
        if (channel.lastMessageAt == null ||
            eventTime.isAfter(channel.lastMessageAt!)) {
          updated[idx] = channel.copyWith(lastMessageAt: eventTime);
        }
      }

      return updated;
    });
  }

  Set<String> _mutedChannelIds() => {
    for (final entry in ref.read(channelMutesProvider).store.channels.entries)
      if (entry.value.muted) entry.key,
  };

  Set<String> _followedRootIds() =>
      ref.read(threadFollowsProvider).followedRootIds;

  void _loadThreadInterestStores(String pubkey) {
    final normalizedPubkey = pubkey.toLowerCase();
    if (_threadInterestPubkey == normalizedPubkey) return;
    _threadInterestPubkey = normalizedPubkey;
    try {
      final prefs = ref.read(savedPrefsProvider);
      _participatedRootIds = _readRootIdSet(
        prefs.getString('$_participatedRootIdsPrefix:$normalizedPubkey'),
      );
      _authoredRootIds = _readRootIdSet(
        prefs.getString('$_authoredRootIdsPrefix:$normalizedPubkey'),
      );
    } catch (_) {
      _participatedRootIds = {};
      _authoredRootIds = {};
    }
  }

  void _recordSelfThreadInterest(NostrEvent event, String pubkey) {
    final ref = event.threadReference;
    final target = ref.rootId != null ? _participatedRootIds : _authoredRootIds;
    final id = ref.rootId ?? event.id;
    if (!target.add(id)) return;
    _writeThreadInterestStores(pubkey);
  }

  void _writeThreadInterestStores(String pubkey) {
    final normalizedPubkey = pubkey.toLowerCase();
    try {
      final prefs = ref.read(savedPrefsProvider);
      prefs.setString(
        '$_participatedRootIdsPrefix:$normalizedPubkey',
        _encodeRootIdSet(_participatedRootIds),
      );
      prefs.setString(
        '$_authoredRootIdsPrefix:$normalizedPubkey',
        _encodeRootIdSet(_authoredRootIds),
      );
    } catch (_) {
      // Ignore storage failures; in-memory interest still works this session.
    }
  }

  void clearObservedUnreadForChannel(String channelId) {
    _latestObservedByChannel.remove(channelId);
    _observedUnreadEventsByChannel.remove(channelId);
    state = state.whenData((channels) => List<Channel>.of(channels));
  }

  void clearObservedUnreadCoveredByRead(String channelId, int readAt) {
    final latest = _latestObservedByChannel[channelId];
    if (latest != null && latest <= readAt) {
      clearObservedUnreadForChannel(channelId);
    }
  }

  /// Backstop refresh that preserves existing state on transient failure.
  Future<void> _backstopRefresh() async {
    try {
      final sessionState = ref.read(relaySessionProvider);
      final prevChannels = state.value ?? const [];
      final prevLastMessage = {
        for (final c in prevChannels)
          if (c.lastMessageAt != null) c.id: c.lastMessageAt,
      };
      final channels = await _fetch(
        subscribeLive: sessionState.status == SessionStatus.connected,
        fetchLastMessage: false,
        fetchDirectory: false,
      );
      for (var i = 0; i < channels.length; i++) {
        final prev = prevLastMessage[channels[i].id];
        if (channels[i].lastMessageAt == null && prev != null) {
          channels[i] = channels[i].copyWith(lastMessageAt: prev);
        }
      }
      state = AsyncData(channels);
    } on _StaleChannelRefresh {
      return;
    } catch (error) {
      debugPrint('[ChannelsNotifier] backstop refresh failed: $error');
    }
  }

  /// Refreshes memberships and, when [fetchDirectory], the open directory.
  /// Invite starter recovery opts in to converge on another identity's
  /// starters instead of attempting duplicate creation from memberships.
  Future<void> refresh({bool fetchDirectory = false}) async {
    final sessionState = ref.read(relaySessionProvider);
    // Don't attempt to fetch when the session isn't connected — fetchHistory
    // would send REQs over an unauthenticated socket that either time out
    // (returning empty results) or get cancelled on disconnect, replacing the
    // cached channel list with [] or an error. Wait for `build()` to re-run
    // when the session transitions to connected.
    if (sessionState.status != SessionStatus.connected) return;
    try {
      final channels = await _fetch(
        subscribeLive: true,
        fetchDirectory: fetchDirectory,
      );
      state = AsyncData(channels);
    } on _StaleChannelRefresh {
      return;
    } catch (error, stackTrace) {
      state = AsyncError(error, stackTrace);
    }
  }

  /// Loads the directory when Browse channels opens after startup or an error.
  Future<void> ensureDirectoryLoaded() async {
    final directoryState = ref.read(channelDirectoryLoadStatusProvider);
    final scope = channelDirectoryScope(
      ref.read(relayConfigProvider).baseUrl,
      ref.read(myPubkeyProvider),
    );
    if (directoryState.scope == scope &&
        (directoryState.status == ChannelDirectoryLoadStatus.loading ||
            directoryState.status == ChannelDirectoryLoadStatus.loaded)) {
      return;
    }
    await retryDirectory();
  }

  /// Retries channel discovery while retaining the current channel list.
  Future<void> retryDirectory() async {
    final previousChannels = state.value;
    final directoryStatus = ref.read(
      channelDirectoryLoadStatusProvider.notifier,
    );
    final scope = channelDirectoryScope(
      ref.read(relayConfigProvider).baseUrl,
      ref.read(myPubkeyProvider),
    );
    final directoryState = ref.read(channelDirectoryLoadStatusProvider);
    if (directoryState.scope == scope &&
        directoryState.status == ChannelDirectoryLoadStatus.loading) {
      return;
    }
    if (ref.read(relaySessionProvider).status != SessionStatus.connected) {
      directoryStatus.markError(scope);
      return;
    }
    try {
      state = AsyncData(
        await _fetch(subscribeLive: true, fetchDirectory: true),
      );
    } on _StaleChannelRefresh {
      // A community or identity switch retired this request. Its response
      // describes a scope the user has left, so write neither the channel list
      // nor the load status; the new scope owns both now.
      return;
    } catch (error, stackTrace) {
      directoryStatus.markError(scope);
      state = previousChannels == null
          ? AsyncError(error, stackTrace)
          : AsyncData(previousChannels);
    }
  }
}

final channelsProvider = AsyncNotifierProvider<ChannelsNotifier, List<Channel>>(
  ChannelsNotifier.new,
);
