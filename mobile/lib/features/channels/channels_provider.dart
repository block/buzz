import 'dart:async';
import 'dart:math';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/utils/string_utils.dart';
import 'channel.dart';
import 'channel_management_provider.dart' show channelDetailsProvider;
import 'read_state/read_state_provider.dart';
import 'unread_badge/is_high_priority_event.dart';
import 'unread_badge/should_notify_for_event.dart';

const _channelTypeOrder = {'stream': 0, 'forum': 1, 'dm': 2};

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
  final Map<String, int> _latestHighPriorityByChannel = {};

  Map<String, int> get latestHighPriorityByChannel =>
      Map.unmodifiable(_latestHighPriorityByChannel);

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
      _latestHighPriorityByChannel.clear();
      _backstopTimer?.cancel();
      _backstopTimer = null;
    });

    if (sessionState.status != SessionStatus.connected) {
      _clearLiveSubscriptions();
      _latestHighPriorityByChannel.clear();
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

    final channels = <Channel>[];
    for (final event in dedupedMetas) {
      final channel = _channelFromMeta(
        event,
        isMember: true,
        displayNames: displayNames,
      );
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
              if (shouldNotifyForEvent(event, myPk)) {
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

    // Backfill high-priority map from recent history so unread @mentions that
    // arrived before app launch are correctly classified as high-priority tier.
    unawaited(_backfillHighPriority(channels));

    _backstopTimer?.cancel();
    _backstopTimer = Timer.periodic(
      _backstopInterval,
      (_) => _backstopRefresh(),
    );
  }

  Future<void> _backfillHighPriority(List<Channel> channels) async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) return;

    final session = ref.read(relaySessionProvider.notifier);
    final readState = ref.read(readStateProvider);
    final futures = <Future<void>>[];

    for (final channel in channels) {
      if (!channel.isMember || channel.isArchived) continue;

      // All DM messages are high-priority — no need to scan event history.
      if (channel.isDm) {
        final lastMsg = channel.lastMessageAt;
        if (lastMsg != null) {
          _latestHighPriorityByChannel[channel.id] =
              lastMsg.millisecondsSinceEpoch ~/ 1000;
        }
        continue;
      }

      final readAt = readState.effectiveTimestamp(channel.id);
      futures.add(
        _backfillHighPriorityForChannel(session, channel, myPk, readAt),
      );
    }

    // Batch into groups of 5 to avoid saturating the relay.
    const batchSize = 5;
    for (var i = 0; i < futures.length; i += batchSize) {
      await Future.wait(futures.sublist(i, min(i + batchSize, futures.length)));
    }

    // Trigger unreadBadgeProvider to re-evaluate now that the map is populated.
    state = state.whenData((channels) => List<Channel>.of(channels));
  }

  Future<void> _backfillHighPriorityForChannel(
    RelaySessionNotifier session,
    Channel channel,
    String myPk,
    int? readAt,
  ) async {
    // For non-DM channels, fetch events since the last read timestamp and scan
    // for high-priority ones. Using `since` avoids fetching messages the user
    // has already seen, and `limit: 200` covers deep mention backfills.
    try {
      final events = await session.fetchHistory(
        NostrFilter(
          kinds: EventKind.channelEventKinds,
          tags: {
            '#h': [channel.id],
          },
          since: readAt ?? 0,
          limit: 200,
        ),
      );

      var maxHighPriority = 0;
      for (final event in events) {
        if (event.pubkey == myPk) continue;
        if (!EventKind.channelMessageEventKinds.contains(event.kind)) continue;
        if (isHighPriorityEvent(event.tags, myPk) &&
            event.createdAt > maxHighPriority) {
          maxHighPriority = event.createdAt;
        }
      }

      if (maxHighPriority > 0) {
        final current = _latestHighPriorityByChannel[channel.id] ?? 0;
        if (maxHighPriority > current) {
          _latestHighPriorityByChannel[channel.id] = maxHighPriority;
        }
      }
    } catch (error) {
      debugPrint(
        '[ChannelsNotifier] backfill failed for ${channel.id}: $error',
      );
    }
  }

  void _handleLiveEvent(NostrEvent event) {
    final channelId = event.channelId;
    if (channelId == null) return;

    final myPk = ref.read(myPubkeyProvider);

    state = state.whenData((channels) {
      final idx = channels.indexWhere((c) => c.id == channelId);
      if (idx == -1) {
        refresh();
        return channels;
      }
      final updated = List<Channel>.of(channels);
      final channel = updated[idx];

      if (myPk != null && shouldNotifyForEvent(event, myPk)) {
        final eventTime = DateTime.fromMillisecondsSinceEpoch(
          event.createdAt * 1000,
          isUtc: true,
        );
        if (channel.lastMessageAt == null ||
            eventTime.isAfter(channel.lastMessageAt!)) {
          updated[idx] = channel.copyWith(lastMessageAt: eventTime);
        }
      }

      if (myPk != null &&
          event.pubkey != myPk &&
          (channel.isDm || isHighPriorityEvent(event.tags, myPk))) {
        final current = _latestHighPriorityByChannel[channelId] ?? 0;
        if (event.createdAt > current) {
          _latestHighPriorityByChannel[channelId] = event.createdAt;
        }
      }

      return updated;
    });
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
