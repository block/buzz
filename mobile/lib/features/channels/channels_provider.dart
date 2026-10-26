import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/channels/channel_list_snapshot.dart';
import '../../shared/theme/theme_provider.dart';
import '../../shared/utils/string_utils.dart';
import 'channel.dart';
import 'channel_management_provider.dart'
    show ChannelMember, channelDetailsProvider;
import 'channel_mutes/channel_mutes_provider.dart';
import 'huddle_channel_filter.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import 'thread_follows/thread_follows_provider.dart';
import 'unread_badge/is_high_priority_event.dart';
import 'unread_badge/observed_unread_event.dart';
import 'unread_badge/should_notify_for_event.dart';

part 'channel_directory.dart';
part 'channel_snapshot.dart';
part 'channel_member_snapshots.dart';
part 'channels_provider_lifecycle.dart';

const _channelTypeOrder = {'stream': 0, 'forum': 1, 'dm': 2};
const _unreadCatchUpLimit = 1000;

/// Relay requests one channel-history batch keeps in flight at most.
const _historyRequestConcurrency = 4;
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
  Future<void> _unreadCatchUp = Future.value();
  int? _settledUnreadGeneration;
  Completer<void> _unreadReadinessChanged = Completer<void>();
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
  bool _hasCachedSnapshot = false;
  bool _needsSnapshotRevalidation = false;
  bool _backgroundRefreshInFlight = false;
  bool _backgroundRefreshQueued = false;
  int _buildGeneration = 0;
  void Function(List<Channel>, Set<String>)? _saveSnapshot;
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

  /// Whether this scope has a reusable list from memory or local storage.
  /// Remains true during background revalidation and later refreshes.
  bool get hasCachedSnapshot => _hasCachedSnapshot;

  /// Waits for the current list's initial unread history to settle.
  ///
  /// Presentation transitions can await this without blocking ordinary cached
  /// channel rendering. Failure keeps observed unread state and permits landing;
  /// follow a newer refresh if it replaces work we awaited.
  Future<void> waitForUnreadCatchUp() async {
    while (ref.mounted) {
      final changed = _unreadReadinessChanged.future;
      final subscriptions = _liveSubscriptionQueue;
      await subscriptions;
      if (!ref.mounted) return;
      final unread = _unreadCatchUp;
      await unread;
      if (identical(subscriptions, _liveSubscriptionQueue) &&
          identical(unread, _unreadCatchUp) &&
          _settledUnreadGeneration == _subscriptionVersion) {
        return;
      }
      // A disconnected generation has no settled destination history.
      // Wake on reconciliation rather than accepting an older completed future.
      await changed;
    }
  }

  void _notifyUnreadReadinessChanged() {
    final previous = _unreadReadinessChanged;
    _unreadReadinessChanged = Completer<void>();
    previous.complete();
  }

  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel =>
      Map<String, Map<String, ObservedUnreadEvent>>.unmodifiable({
        for (final entry in _observedUnreadEventsByChannel.entries)
          entry.key: Map<String, ObservedUnreadEvent>.unmodifiable(entry.value),
      });

  @override
  Future<List<Channel>> build() async {
    final buildGeneration = ++_buildGeneration;
    _backgroundRefreshInFlight = false;
    _backgroundRefreshQueued = false;
    _refreshCoordinator.retireInFlight();
    final relayBaseUrl = ref.watch(relayConfigProvider).baseUrl;
    final pubkey = ref.watch(myPubkeyProvider)?.toLowerCase();
    if (_memberSnapshotRelayBaseUrl != relayBaseUrl ||
        _memberSnapshotPubkey != pubkey) {
      // A cached snapshot is usable only within its relay and identity.
      // Never reuse another community or identity’s in-memory content.
      _hasLoaded = false;
      _hasCachedSnapshot = false;
      _needsSnapshotRevalidation = false;
      _memberSnapshotRelayBaseUrl = relayBaseUrl;
      _memberSnapshotPubkey = pubkey;
      _memberSnapshotsByChannelId = const {};
      _directoryMetas = const [];
      _hiddenDmIds = const {};
      // Retire any in-flight directory request: its response describes the
      // previous relay or identity and must not reach this scope's state.
      _refreshCoordinator.retireInFlight();
    }
    final snapshot =
        (_hasLoaded ? state.value : null) ??
        _restoreSnapshot(pubkey, relayBaseUrl);
    if (snapshot != null) _hasCachedSnapshot = true;
    _saveSnapshot = pubkey == null
        ? null
        : _captureSnapshotWriter(pubkey, relayBaseUrl);
    final connected = Completer<void>();
    var connectionVersion = 0;
    final sessionState = ref.read(relaySessionProvider);
    final waitingForInitialConnection =
        sessionState.status != SessionStatus.connected;
    ref.listen(relaySessionProvider, (previous, next) {
      if (next.status != SessionStatus.connected) return;
      if (previous?.status != SessionStatus.connected) connectionVersion++;
      if (waitingForInitialConnection && !connected.isCompleted) {
        connected.complete();
      } else if (previous?.status != SessionStatus.connected) {
        // A new authenticated connection can change relay admission policy.
        // Ordinary refreshes must not retry an unchanged terminal rejection.
        _terminallyClosedLiveChunks.clear();
        unawaited(_refreshInBackground(queueIfBusy: true));
      }
    });

    // Re-fetch when the app returns to foreground so channels created on
    // another device while mobile was backgrounded appear immediately.
    ref.listen(appLifecycleProvider, (prev, next) {
      if (next != AppLifecycleState.resumed) return;
      // Resume is published before the session hears it. Let the session
      // decide first: one that reconnects reconciles when it connects, so
      // refreshing here too would fetch the same list twice.
      final versionBeforeResume = connectionVersion;
      scheduleMicrotask(() {
        if (!ref.mounted) return;
        // A fast reconnect may already have reconciled before this callback.
        if (connectionVersion == versionBeforeResume &&
            ref.read(relaySessionProvider).status == SessionStatus.connected) {
          refresh();
        }
      });
    });

    ref.onDispose(() {
      _buildGeneration++;
      _refreshCoordinator.retireInFlight();
      _clearLiveSubscriptions();
      _latestObservedByChannel.clear();
      _observedUnreadEventsByChannel.clear();
      _backstopTimer?.cancel();
      _backstopTimer = null;
    });

    if (snapshot != null) {
      _startBackstopTimer();
      // Install local data before starting work that can notify listeners.
      unawaited(
        Future<void>.microtask(() async {
          if (waitingForInitialConnection) await connected.future;
          if (!ref.mounted || buildGeneration != _buildGeneration) return;
          await _refreshInBackground();
        }),
      );
      return snapshot;
    }

    if (sessionState.status != SessionStatus.connected) {
      await connected.future;
    }
    if (!ref.mounted || buildGeneration != _buildGeneration) {
      throw const _StaleChannelRefresh();
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
    _loadThreadInterestStores(myPk);
    final saveSnapshot = _saveSnapshot;

    final session = ref.read(relaySessionProvider.notifier);

    // Acquire request ownership before the first relay await. Every channel-list
    // path uses this fence so completion order cannot let an older ordinary,
    // directory, or reconnect refresh replace a newer membership list.
    final fence = await _refreshCoordinator.beginRefresh(
      fetchesDirectory: fetchDirectory,
    );

    // Step 1: find the channels I'm a member of via kind:39002.
    final memberships = _latestChannelEvents(
      await _fenced(fence, _fetchChannelMemberships(session, myPk)),
      kind: 39002,
    );
    final memberChannelIds = memberships
        .map((e) => e.getTagValue('d'))
        .whereType<String>()
        .toSet();

    // Step 2: everything below the membership list depends only on it, so
    // send those reads together instead of one round trip after another. A
    // user with no memberships must still continue to directory discovery.
    final memberChannelIdList = memberChannelIds.toList();
    final memberMetasRequest = memberChannelIds.isEmpty
        ? Future.value(const <NostrEvent>[])
        : session.fetchHistory(
            NostrFilters.channelMetadata(memberChannelIdList),
          );
    final hiddenDmIdsRequest = _fetchHiddenDmIds(session, myPk);
    final huddleStartsRequest = _fetchHuddleStarts(
      session,
      memberChannelIdList,
    );
    final (memberMetas, hiddenDmIds, huddleStarts) = await _fenced(
      fence,
      () async {
        // Future.wait handles every request's failure and rethrows the first
        // one unchanged, as soon as it happens.
        await Future.wait(eagerError: true, [
          memberMetasRequest,
          hiddenDmIdsRequest,
          huddleStartsRequest,
        ]);
        return (
          await memberMetasRequest,
          await hiddenDmIdsRequest,
          await huddleStartsRequest,
        );
      }(),
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
    final dedupedMetas = _latestChannelEvents([
      ...memberMetas,
      ..._directoryMetas,
    ], kind: 39000);
    final parsedMetas = {
      for (final event in dedupedMetas) event: ChannelData.fromEvent(event),
    };
    // Membership discovery already returned the full authoritative rosters.
    // Their owner roles validate Huddle links without downloading them again.
    final huddleBackingIds = huddleBackingChannelIds(huddleStarts, memberships);
    final visibleMetas = parsedMetas.entries.where((entry) {
      final data = entry.value;
      final isMember = memberChannelIds.contains(data.id);
      if (!isMember &&
          (data.visibility == 'private' || data.channelType == 'dm')) {
        return false;
      }
      if (data.channelType == 'dm' && hiddenDmIds.contains(data.id)) {
        return false;
      }
      return !(huddleBackingIds.contains(data.id) &&
          data.channelType == 'stream' &&
          data.visibility == 'private');
    }).toList();

    // Resolve DM participant display names. Extracted into the part file so
    // `channels_provider.dart` stays under the 1200-line ceiling enforced by
    // `just file-size-check`.
    final displayNamesRequest = _resolveDmDisplayNames(
      ref,
      fence,
      parsedMetas.values,
      myPk,
      refreshCached: fetchLastMessage,
    );
    // Latest-message filters depend on channel identity/privacy, not names.
    // Start them alongside profiles, and observe both errors immediately.
    final latestEventsRequest = fetchLastMessage
        ? _fetchLastMessageEvents(session, [
            for (final entry in visibleMetas)
              if (memberChannelIds.contains(entry.value.id) &&
                  !entry.value.isArchived)
                entry.value,
          ])
        : Future.value(const <NostrEvent>[]);
    final (displayNames, latestEvents) = await _fenced(fence, () async {
      await Future.wait(eagerError: true, [
        displayNamesRequest,
        latestEventsRequest,
      ]);
      return (await displayNamesRequest, await latestEventsRequest);
    }());
    // Name resolution may return without a fenced read; check before writing.
    fence.ensureCurrent();
    _cacheMemberSnapshots(memberships, replaceAll: true);
    _hiddenDmIds = Set.unmodifiable(hiddenDmIds);

    final channels = <Channel>[];
    for (final entry in visibleMetas) {
      final id = entry.value.id;
      final isMember = memberChannelIds.contains(id);
      final channel = _channelFromMeta(
        entry.key,
        entry.value,
        isMember: isMember,
        displayNames: displayNames,
      );
      // Ephemeral (TTL) channels are surfaced in the list with an
      // `_EphemeralBadge` rendered in `channels_page.dart` — they shouldn't be
      // hidden. Desktop shows them too. Previously dropped here unconditionally,
      // which made TTL channels invisible on iOS even when the user was a member.
      channels.add(channel);
    }

    // Use the membership snapshots already fetched above for both Huddle
    // linkage validation and member-count hydration.
    final memberCounts = _memberCountsByChannelId(memberships);
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
      final events = latestEvents;
      final lastMessageMap = <String, int>{};
      // An unavailable batch keeps the timestamps already known rather than
      // installing channels whose latest message looks absent.
      if (events == null) {
        for (final channel in state.value ?? const <Channel>[]) {
          final at = channel.lastMessageAt;
          if (at != null) {
            lastMessageMap[channel.id] = at.millisecondsSinceEpoch ~/ 1000;
          }
        }
      }
      final mutedChannelIds = _mutedChannelIds();
      for (final event in events ?? const <NostrEvent>[]) {
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

    // Details are a separate cache. Invalidate changes that would otherwise
    // overwrite fresh Solo identity or archive state when opening a channel.
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
      if (prev == null ||
          prev.isArchived != channel.isArchived ||
          prev.channelType != channel.channelType ||
          prev.visibility != channel.visibility ||
          prev.createdBy != channel.createdBy ||
          !listEquals(prev.labels, channel.labels)) {
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
    if (!fetchLastMessage) {
      // Backstop metadata refreshes retain live timestamps in the persisted
      // snapshot as well as the visible list.
      for (var i = 0; i < channels.length; i++) {
        final previous = prevById[channels[i].id]?.lastMessageAt;
        if (previous != null) {
          channels[i] = channels[i].copyWith(lastMessageAt: previous);
        }
      }
    }
    if (fetchLastMessage) _needsSnapshotRevalidation = latestEvents == null;
    saveSnapshot?.call(channels, _hiddenDmIds);
    return channels;
  }

  /// Fetches each channel's independent latest-message window in one HTTP
  /// bridge request. The relay preserves NIP-01 per-filter limits while
  /// executing the filters with bounded concurrency, avoiding an unbounded
  /// burst of websocket REQs on communities with many channels.
  Future<List<NostrEvent>?> _fetchLastMessageEvents(
    RelaySessionNotifier session,
    List<ChannelData> channels,
  ) async {
    if (channels.isEmpty) return const [];

    final filters = [
      for (final channel in channels)
        NostrFilter(
          kinds: EventKind.channelMessageEventKinds,
          tags: {
            '#h': [channel.id],
          },
          limit: channel.channelType == 'dm' ? 1 : 20,
        ),
    ];

    return _fetchChannelHistoryBatch(
      session,
      filters,
      operation: 'latest-message query',
    );
  }

  /// Returns null when the batch is unavailable: it hit a relay deadline.
  ///
  /// The relay rejects a request whose filters name more than
  /// [kMaxExplicitChannelValues] channels in aggregate, so larger batches are
  /// split into requests within that cap.
  Future<List<NostrEvent>?> _fetchChannelHistoryBatch(
    RelaySessionNotifier session,
    List<NostrFilter> filters, {
    required String operation,
  }) async {
    final requests = <List<NostrFilter>>[];
    var channelValues = 0;
    for (final filter in filters) {
      final values = filter.tags['#h']?.length ?? 0;
      if (requests.isEmpty ||
          channelValues + values > kMaxExplicitChannelValues) {
        requests.add([]);
        channelValues = 0;
      }
      requests.last.add(filter);
      channelValues += values;
    }
    final events = <NostrEvent>[];
    final fallback = <NostrFilter>[];
    var deadline = false;
    for (
      var start = 0;
      start < requests.length;
      start += _historyRequestConcurrency
    ) {
      final wave = requests.sublist(
        start,
        min(start + _historyRequestConcurrency, requests.length),
      );
      final results = await Future.wait(
        wave.map((request) async {
          try {
            return await session.queryRelay(request);
          } catch (error) {
            // Per-filter fallback would re-run the timed-out work another way.
            if (isRelayDeadlineError(error)) {
              deadline = true;
            } else {
              debugPrint(
                '[ChannelsNotifier] batched $operation failed; '
                'using bounded websocket fallback: $error',
              );
            }
            return null;
          }
        }),
      );
      // A request that hit the deadline makes the whole batch unavailable.
      if (deadline) {
        debugPrint('[ChannelsNotifier] batched $operation hit the deadline');
        return null;
      }
      for (final (index, result) in results.indexed) {
        result == null ? fallback.addAll(wave[index]) : events.addAll(result);
      }
    }
    if (fallback.isEmpty) return events;

    // Fallback starts once every request has settled, so the batch never has
    // more than the request concurrency in flight.
    final fallbackEvents = await _fetchChannelHistoryFallback(
      session,
      fallback,
    );
    return fallbackEvents == null ? null : [...events, ...fallbackEvents];
  }

  Future<List<NostrEvent>?> _fetchChannelHistoryFallback(
    RelaySessionNotifier session,
    List<NostrFilter> filters,
  ) async {
    final events = <NostrEvent>[];
    var deadline = false;
    for (
      var start = 0;
      start < filters.length;
      start += _historyRequestConcurrency
    ) {
      // Stop sending once a filter hit the deadline; partial events are not
      // a result.
      if (deadline) return null;
      final end = min(start + _historyRequestConcurrency, filters.length);
      final results = await Future.wait(
        filters.sublist(start, end).map((filter) async {
          try {
            return await session.fetchHistory(filter);
          } catch (error) {
            // A timed-out filter makes the batch unavailable, not empty.
            if (isRelayDeadlineError(error)) deadline = true;
            return const <NostrEvent>[];
          }
        }),
      );
      for (final result in results) {
        events.addAll(result);
      }
    }
    if (deadline) return null;
    return events;
  }

  /// Build a [Channel] from a kind:39000 metadata event.
  ///
  /// [displayNames] maps lowercase participant pubkey → resolved label and is
  /// used to populate [Channel.participants] for DMs so [Channel.displayLabel]
  /// can render real names instead of the relay-canonical "DM" name.
  Channel _channelFromMeta(
    NostrEvent event,
    ChannelData data, {
    required bool isMember,
    Map<String, String> displayNames = const {},
  }) {
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
      labels: data.labels,
      visibility: data.visibility,
      description: data.description,
      topic: data.topic,
      createdBy: data.creator ?? event.pubkey,
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
    try {
      final myPk = ref.read(myPubkeyProvider);
      if (myPk == null) return;

      final session = ref.read(relaySessionProvider.notifier);
      final mutedChannelIds = _mutedChannelIds();
      final readState = ref.read(readStateProvider);
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

      final events = await _fetchChannelHistoryBatch(
        session,
        filters,
        operation: 'unread catch-up',
      );
      // Unavailable: keep the unread state already observed.
      if (events == null) return;
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

      final recordedAtByChannel = <String, int>{};
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
        recordedAtByChannel[channelId] = max(
          recordedAtByChannel[channelId] ?? 0,
          event.createdAt,
        );
      }
      // Republish only when this catch-up actually changed unread state. A
      // batch that recorded nothing has nothing to show, and a failed or
      // superseded batch must not repaint another refresh's list: the retired
      // check above already returned in that case, and no await separates it
      // from here, so a second check would be dead code.
      // Recorded events also advance `lastMessageAt`, as live events do: a
      // deadline-unavailable latest-message batch leaves it unset.
      if (recordedAtByChannel.isNotEmpty) {
        state = state.whenData(
          (channels) => [
            for (final channel in channels)
              _advanceLastMessageAt(channel, recordedAtByChannel[channel.id]),
          ],
        );
      }
    } catch (error) {
      if (!ref.mounted) return;
      debugPrint('[ChannelsNotifier] unread catch-up failed: $error');
    } finally {
      // Optional history can terminate without data. Release this destination
      // with its retained unread state, but never settle a retired request.
      if (ref.mounted && !_isCatchUpRetired(fence, subscriptionGeneration)) {
        _settledUnreadGeneration = subscriptionGeneration;
        _notifyUnreadReadinessChanged();
      }
    }
  }

  static Channel _advanceLastMessageAt(Channel channel, int? createdAt) {
    if (createdAt == null) return channel;
    final at = DateTime.fromMillisecondsSinceEpoch(
      createdAt * 1000,
      isUtc: true,
    );
    final current = channel.lastMessageAt;
    if (current != null && !at.isAfter(current)) return channel;
    return channel.copyWith(lastMessageAt: at);
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
      final channels = await _fetch(
        subscribeLive: sessionState.status == SessionStatus.connected,
        fetchLastMessage: false,
        fetchDirectory: false,
      );
      state = AsyncData(channels);
    } on _StaleChannelRefresh {
      return;
    } catch (error) {
      debugPrint('[ChannelsNotifier] backstop refresh failed: $error');
    }
  }

  /// Fetches current memberships for decisions that cannot use cached data.
  ///
  /// Unlike background refresh, failures (including disconnection or a retired
  /// request) propagate. Cached UI state stays visible while the request runs.
  Future<List<Channel>> fetchFresh({bool fetchDirectory = false}) async {
    final generation = _buildGeneration;
    try {
      await future;
    } catch (_) {
      // A failed initial list must not prevent an explicit network retry.
    }
    if (!ref.mounted || generation != _buildGeneration) {
      throw const _StaleChannelRefresh();
    }
    if (ref.read(relaySessionProvider).status != SessionStatus.connected) {
      throw StateError('Connect to the community before refreshing channels.');
    }
    final channels = await _fetch(
      subscribeLive: true,
      fetchDirectory: fetchDirectory,
    );
    state = AsyncData(channels);
    return channels;
  }

  /// Best-effort refresh of memberships and, optionally, the open directory.
  /// Keeps usable UI state on failure. Decisions requiring fresh membership
  /// must use [fetchFresh] to receive the fetch error instead.
  Future<void> refresh({bool fetchDirectory = false}) async {
    final sessionState = ref.read(relaySessionProvider);
    // Don't attempt to fetch when the session isn't connected — fetchHistory
    // would send REQs over an unauthenticated socket that either time out
    // (returning empty results) or get cancelled on disconnect, replacing the
    // cached channel list with [] or an error. Wait for `build()` to re-run
    // when the session transitions to connected.
    if (sessionState.status != SessionStatus.connected) return;
    try {
      await fetchFresh(fetchDirectory: fetchDirectory);
    } on _StaleChannelRefresh {
      return;
    } catch (error, stackTrace) {
      // A refresh failure cannot make already-fetched content disappear.
      if (state.value == null) {
        state = AsyncError(error, stackTrace);
      } else {
        debugPrint(
          '[ChannelsNotifier] refresh failed; retaining cached list: $error',
        );
      }
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
