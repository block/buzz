import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme_provider.dart';
import '../../shared/utils/string_utils.dart';
import 'channel.dart';
import 'channel_management_provider.dart' show channelDetailsProvider;
import 'channel_mutes/channel_mutes_provider.dart';
import 'read_state/read_state_provider.dart';
import 'unread_badge/is_high_priority_event.dart';
import 'unread_badge/observed_unread_event.dart';
import 'unread_badge/should_notify_for_event.dart';

const _channelTypeOrder = {'stream': 0, 'forum': 1, 'dm': 2};
const _unreadCatchUpLimit = 1000;
const _participatedRootIdsPrefix = 'buzz-thread-participation.v1';
const _authoredRootIdsPrefix = 'buzz-thread-authored.v1';

/// Loads the user's channel list from the relay over WebSocket.
///
/// Two-step query:
///   1. Fetch kind:39002 membership events tagged `#p:<my-pubkey>` to find
///      the channel ids I'm a member of.
///   2. Fetch the corresponding kind:39000 channel metadata events.
///
/// Live updates are layered on top via per-channel subscriptions on the
/// `#h` tag for any of the visible channel event kinds — incoming events
/// bump `lastMessageAt` for that channel.
class ChannelsNotifier extends AsyncNotifier<List<Channel>> {
  static const _backstopInterval = Duration(seconds: 60);

  final List<void Function()> _unsubscribers = [];
  int _subscriptionVersion = 0;
  Timer? _backstopTimer;
  final Map<String, int> _latestObservedByChannel = {};
  final Map<String, Map<String, ObservedUnreadEvent>>
  _observedUnreadEventsByChannel = {};
  Set<String> _participatedRootIds = {};
  Set<String> _authoredRootIds = {};
  String? _threadInterestPubkey;

  Map<String, int> get latestObservedByChannel =>
      Map.unmodifiable(_latestObservedByChannel);

  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel =>
      Map<String, Map<String, ObservedUnreadEvent>>.unmodifiable({
        for (final entry in _observedUnreadEventsByChannel.entries)
          entry.key: Map<String, ObservedUnreadEvent>.unmodifiable(entry.value),
      });

  @override
  Future<List<Channel>> build() {
    final sessionState = ref.watch(relaySessionProvider);
    ref.watch(relayConfigProvider);

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
      _clearLiveSubscriptions();
      _latestObservedByChannel.clear();
      _observedUnreadEventsByChannel.clear();
      // Preserve the last successfully loaded channels while reconnecting
      // instead of re-entering a loading/error state. The UI will show cached
      // channels with a "Reconnecting…" banner overlay, which is far better
      // than a blank screen.
      final previous = state.value;
      if (previous != null && previous.isNotEmpty) {
        return Future.value(previous);
      }
    }

    return _fetch(
      subscribeLive: sessionState.status == SessionStatus.connected,
    );
  }

  Future<List<Channel>> _fetch({
    bool subscribeLive = false,
    bool fetchLastMessage = true,
  }) async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) throw StateError('No signing identity available');
    _loadThreadInterestStores(myPk);

    final session = ref.read(relaySessionProvider.notifier);

    // Step 1: find the channels I'm a member of via kind:39002.
    final memberships = <NostrEvent>[];
    {
      int? until;
      const pageSize = 500;
      while (true) {
        final page = await session.fetchHistory(
          NostrFilter(
            kinds: const [39002],
            tags: {
              '#p': [myPk],
            },
            limit: pageSize,
            until: until,
          ),
        );
        memberships.addAll(page);
        if (page.length < pageSize) break;
        until = page.map((e) => e.createdAt).reduce(min) - 1;
      }
    }
    final channelIds = memberships
        .map((e) => e.getTagValue('d'))
        .whereType<String>()
        .toSet()
        .toList();
    if (channelIds.isEmpty) return const [];

    // Step 2: pull channel metadata in one batched filter.
    final metas = await session.fetchHistory(
      NostrFilters.channelMetadata(channelIds),
    );

    // Dedupe by `d` tag (channel id) — kind:39000 is parameterized-replaceable,
    // so logically there's exactly one current event per id, but stale revisions
    // from before the relay's d_tag backfill can linger. Keep the highest
    // `created_at` per id so the latest channel_type / name wins.
    final latestMetaPerId = <String, NostrEvent>{};
    for (final event in metas) {
      if (event.kind != 39000) continue;
      final id = event.getTagValue('d');
      if (id == null) continue;
      final existing = latestMetaPerId[id];
      if (existing == null || event.createdAt > existing.createdAt) {
        latestMetaPerId[id] = event;
      }
    }
    final dedupedMetas = latestMetaPerId.values;

    // Resolve DM participant display names. Relay stores DM channels with
    // literal name="DM"; pure-Nostr architecture pushes name resolution to
    // the client, so collect non-self participant pubkeys across all DM
    // metas and batch-fetch their kind:0 profiles in one round-trip.
    final dmParticipants = <String>{};
    final myPkLower = myPk.toLowerCase();
    for (final event in dedupedMetas) {
      final data = ChannelData.fromEvent(event);
      if (data.channelType != 'dm') continue;
      for (final pk in data.participantPubkeys) {
        final lower = pk.toLowerCase();
        if (lower != myPkLower) dmParticipants.add(lower);
      }
    }

    final displayNames = <String, String>{};
    if (dmParticipants.isNotEmpty) {
      final profileEvents = await session.fetchHistory(
        NostrFilters.profilesBatch(dmParticipants.toList()),
      );
      for (final event in profileEvents) {
        if (event.kind != 0) continue;
        final profile = ProfileData.fromEvent(event);
        final label = profile.displayName?.trim().isNotEmpty == true
            ? profile.displayName!.trim()
            : profile.nip05?.trim().isNotEmpty == true
            ? profile.nip05!.trim()
            : shortPubkey(profile.pubkey);
        displayNames[profile.pubkey.toLowerCase()] = label;
      }
    }

    final hiddenDmIds = await _fetchHiddenDmIds(session, myPk);

    final channels = <Channel>[];
    for (final event in dedupedMetas) {
      final channel = _channelFromMeta(
        event,
        isMember: true,
        displayNames: displayNames,
      );
      if (channel.isDm && hiddenDmIds.contains(channel.id)) continue;
      // Ephemeral (TTL) channels are surfaced in the list with an
      // `_EphemeralBadge` rendered in `channels_page.dart` — they shouldn't be
      // hidden. Desktop shows them too. Previously dropped here unconditionally,
      // which made TTL channels invisible on iOS even when the user was a member.
      channels.add(channel);
    }

    // Batch-fetch member counts via kind:39002 membership events.
    final memberEvents = await session.fetchHistory(
      NostrFilter(
        kinds: const [39002],
        tags: {'#d': channelIds},
        limit: channelIds.length,
      ),
    );
    final memberCounts = <String, int>{};
    for (final event in memberEvents) {
      final chId = event.getTagValue('d');
      if (chId == null) continue;
      final pTags = <String>{};
      for (final tag in event.tags) {
        if (tag.isNotEmpty && tag[0] == 'p' && tag.length > 1) {
          pTags.add(tag[1].toLowerCase());
        }
      }
      memberCounts[chId] = pTags.length;
    }
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
      final lastMessageResults = await Future.wait(
        channels.map((channel) async {
          if (!channel.isMember || channel.isArchived) return null;
          try {
            if (channel.isDm) {
              final events = await session.fetchHistory(
                NostrFilter(
                  kinds: EventKind.channelMessageEventKinds,
                  tags: {
                    '#h': [channel.id],
                  },
                  limit: 1,
                ),
              );
              if (events.isEmpty) return null;
              return MapEntry(channel.id, events.first.createdAt);
            }
            final events = await session.fetchHistory(
              NostrFilter(
                kinds: EventKind.channelMessageEventKinds,
                tags: {
                  '#h': [channel.id],
                },
                limit: 20,
              ),
            );
            for (final event in events) {
              if (shouldNotifyForEvent(
                event,
                myPk,
                mutedChannelIds: _mutedChannelIds(),
                channelId: channel.id,
              )) {
                return MapEntry(channel.id, event.createdAt);
              }
            }
            return null;
          } catch (_) {
            return null;
          }
        }),
      );

      final lastMessageMap = <String, int>{};
      for (final entry
          in lastMessageResults.whereType<MapEntry<String, int>>()) {
        lastMessageMap[entry.key] = entry.value;
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
    final prevById = <String, Channel>{
      for (final c in state.value ?? const <Channel>[]) c.id: c,
    };
    for (final channel in channels) {
      final prev = prevById[channel.id];
      if (prev != null && prev.isArchived != channel.isArchived) {
        ref.invalidate(channelDetailsProvider(channel.id));
      }
    }

    if (subscribeLive) {
      await _subscribeLive(channels);
    }
    return channels;
  }

  Future<Set<String>> _fetchHiddenDmIds(
    RelaySessionNotifier session,
    String myPk,
  ) async {
    try {
      final events = await session.fetchHistory(NostrFilters.hiddenDms(myPk));
      if (events.isEmpty) return const {};
      NostrEvent latest = events.first;
      for (final event in events.skip(1)) {
        if (event.createdAt > latest.createdAt) {
          latest = event;
        }
      }
      return {
        for (final tag in latest.tags)
          if (tag.length >= 2 && tag[0] == 'h') tag[1],
      };
    } catch (_) {
      return const {};
    }
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

  /// Subscribe per-channel to live events (requires `#h` tag for relay
  /// channel-scoped fan-out). Also starts a 60s WS backstop poll to detect
  /// newly created channels we don't yet have subscriptions for.
  Future<void> _subscribeLive(List<Channel> channels) async {
    _clearLiveSubscriptions();
    final subscriptionVersion = _subscriptionVersion;
    if (ref.read(relaySessionProvider).status != SessionStatus.connected) {
      return;
    }

    final session = ref.read(relaySessionProvider.notifier);
    final channelIds = {
      for (final channel in channels)
        if (channel.isMember && !channel.isArchived) channel.id,
    };

    final subscriptions = await Future.wait(
      channelIds.map((channelId) async {
        try {
          return await session.subscribe(
            NostrFilter(
              kinds: EventKind.channelEventKinds,
              tags: {
                '#h': [channelId],
              },
              limit: 0,
            ),
            _handleLiveEvent,
          );
        } catch (error) {
          debugPrint(
            '[ChannelsNotifier] live subscription failed for $channelId: $error',
          );
          return null;
        }
      }),
    );

    if (subscriptionVersion != _subscriptionVersion ||
        ref.read(relaySessionProvider).status != SessionStatus.connected) {
      for (final unsubscribe in subscriptions.whereType<void Function()>()) {
        unsubscribe();
      }
      return;
    }

    _unsubscribers.addAll(subscriptions.whereType<void Function()>());

    unawaited(_catchUpUnreadEvents(channels));

    _backstopTimer?.cancel();
    _backstopTimer = Timer.periodic(
      _backstopInterval,
      (_) => _backstopRefresh(),
    );
  }

  Future<void> _catchUpUnreadEvents(List<Channel> channels) async {
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
    final futures = <Future<void>>[];

    for (final channel in channels) {
      if (!channel.isMember || channel.isArchived) continue;
      final readAt = readState.effectiveTimestamp(channel.id);
      futures.add(
        _catchUpUnreadEventsForChannel(
          session,
          channel,
          myPk,
          readAt,
          mutedChannelIds,
        ),
      );
    }

    const batchSize = 5;
    for (var i = 0; i < futures.length; i += batchSize) {
      await Future.wait(futures.sublist(i, min(i + batchSize, futures.length)));
    }

    state = state.whenData((channels) => List<Channel>.of(channels));
  }

  Future<void> _catchUpUnreadEventsForChannel(
    RelaySessionNotifier session,
    Channel channel,
    String myPk,
    int? readAt,
    Set<String> mutedChannelIds,
  ) async {
    try {
      final events = await session.fetchHistory(
        NostrFilter(
          kinds: EventKind.channelMessageEventKinds,
          tags: {
            '#h': [channel.id],
          },
          since: readAt == null ? 0 : readAt + 1,
          limit: _unreadCatchUpLimit,
        ),
      );

      for (final event in events) {
        if (event.pubkey.toLowerCase() == myPk.toLowerCase()) {
          _recordSelfThreadInterest(event, myPk);
        }
      }

      for (final event in events) {
        if (event.pubkey.toLowerCase() == myPk.toLowerCase()) continue;
        if (readAt != null && event.createdAt <= readAt) continue;
        if (!shouldNotifyForEvent(
          event,
          myPk,
          participatedRootIds: _participatedRootIds,
          authoredRootIds: _authoredRootIds,
          mutedChannelIds: mutedChannelIds,
          channelId: channel.id,
        )) {
          continue;
        }
        _recordUnreadEvent(channel, event, myPk);
      }
    } catch (error) {
      debugPrint(
        '[ChannelsNotifier] unread catch-up failed for ${channel.id}: $error',
      );
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

  void _recordUnreadEvent(Channel channel, NostrEvent event, String myPk) {
    final isThreadedReply =
        event.threadReference.parentId != null && !_isBroadcastReply(event);
    final isHighPriority =
        channel.isDm || isHighPriorityEvent(event.tags, myPk);
    recordObservedUnreadEvent(
      _observedUnreadEventsByChannel,
      channel.id,
      makeObservedUnreadEvent(
        id: event.id,
        createdAt: event.createdAt,
        rootId: _observedUnreadRootId(event),
        highPriority: isHighPriority,
        channelType: channel.channelType,
        isThreadedReply: isThreadedReply,
      ),
      _unreadCatchUpLimit,
    );

    final current = _latestObservedByChannel[channel.id] ?? 0;
    if (event.createdAt > current) {
      _latestObservedByChannel[channel.id] = event.createdAt;
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
      );
      for (var i = 0; i < channels.length; i++) {
        final prev = prevLastMessage[channels[i].id];
        if (channels[i].lastMessageAt == null && prev != null) {
          channels[i] = channels[i].copyWith(lastMessageAt: prev);
        }
      }
      state = AsyncData(channels);
    } catch (error) {
      debugPrint('[ChannelsNotifier] backstop refresh failed: $error');
    }
  }

  Future<void> refresh() async {
    final sessionState = ref.read(relaySessionProvider);
    // Don't attempt to fetch when the session isn't connected — fetchHistory
    // would send REQs over an unauthenticated socket that either time out
    // (returning empty results) or get cancelled on disconnect, replacing the
    // cached channel list with [] or an error. Wait for `build()` to re-run
    // when the session transitions to connected.
    if (sessionState.status != SessionStatus.connected) return;
    state = await AsyncValue.guard(() => _fetch(subscribeLive: true));
  }

  void _clearLiveSubscriptions() {
    _subscriptionVersion++;
    for (final unsubscribe in _unsubscribers) {
      unsubscribe();
    }
    _unsubscribers.clear();
    _backstopTimer?.cancel();
    _backstopTimer = null;
  }
}

final channelsProvider = AsyncNotifierProvider<ChannelsNotifier, List<Channel>>(
  ChannelsNotifier.new,
);

String? _observedUnreadRootId(NostrEvent event) =>
    _isBroadcastReply(event) ? null : event.threadReference.rootId;

bool _isBroadcastReply(NostrEvent event) => event.tags.any(
  (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
);

Set<String> _readRootIdSet(String? raw) {
  if (raw == null || raw.isEmpty) return {};
  try {
    final decoded = jsonDecode(raw);
    if (decoded is! List) return {};
    return {
      for (final value in decoded)
        if (value is String) value,
    };
  } catch (_) {
    return {};
  }
}

String _encodeRootIdSet(Set<String> values) => jsonEncode(values.toList());
