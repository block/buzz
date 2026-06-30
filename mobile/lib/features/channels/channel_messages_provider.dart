import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'channel_management_provider.dart';

/// Provides the message list for a specific channel. Registers a live
/// subscription first, then syncs history via the websocket session.
class ChannelMessagesNotifier extends Notifier<AsyncValue<List<NostrEvent>>> {
  final String channelId;
  void Function()? _unsubscribe;
  bool _reachedOldest = false;
  bool _initInFlight = false;
  int _initVersion = 0;

  ChannelMessagesNotifier(this.channelId);

  /// Last successfully loaded messages, preserved across reconnections so the
  /// UI can show stale data instead of a blank loading spinner.
  List<NostrEvent>? _lastKnownMessages;

  @override
  AsyncValue<List<NostrEvent>> build() {
    final sessionState = ref.watch(relaySessionProvider);
    ref.onDispose(() {
      _initVersion++;
      _clearSubscription();
    });

    if (sessionState.status != SessionStatus.connected) {
      _initVersion++;
      _initInFlight = false;
      // Return cached messages if available so the UI remains usable while
      // disconnected/reconnecting, instead of showing an empty screen.
      return AsyncData(_lastKnownMessages ?? const []);
    }

    // Reset pagination state on rebuild (e.g. after reconnect).
    _reachedOldest = false;
    _init();
    // Show previous messages while fetching fresh ones, instead of a spinner.
    if (_lastKnownMessages case final cached? when cached.isNotEmpty) {
      return AsyncData(cached);
    }
    return const AsyncLoading();
  }

  Future<void> _init() async {
    final initVersion = ++_initVersion;
    _initInFlight = true;
    _clearSubscription();
    try {
      final session = ref.read(relaySessionProvider.notifier);

      // Register live first, then sync history. This matches desktop and closes
      // the race where an event can arrive after history EOSE but before live
      // subscription registration.
      try {
        final unsubscribe = await session.subscribe(
          NostrFilter(
            kinds: EventKind.channelEventKinds,
            tags: {
              '#h': [channelId],
            },
            limit: 200,
          ),
          _handleLiveEvent,
        );
        if (!_isCurrentInit(initVersion)) {
          unsubscribe();
          return;
        }
        _unsubscribe = unsubscribe;
      } catch (error) {
        if (!_isCurrentInit(initVersion)) {
          return;
        }
        debugPrint(
          '[ChannelMessagesNotifier] live subscription failed for $channelId: $error',
        );
      }

      // Fetch recent history via REQ/EOSE after the subscription is active.
      final history = await session.fetchHistory(
        NostrFilter(
          kinds: EventKind.channelEventKinds,
          tags: {
            '#h': [channelId],
          },
          limit: 200,
        ),
      );
      if (!_isCurrentInit(initVersion)) {
        return;
      }

      // Merge fresh history with any events already in state (e.g. from
      // fetchOlder() or live events that arrived while _init was in flight)
      // to avoid discarding data the user has already scrolled through.
      final existing = state.value ?? const [];
      final existingIds = existing.map((e) => e.id).toSet();
      final newEvents = history
          .where((e) => !existingIds.contains(e.id))
          .toList();
      final merged = [...existing, ...newEvents];
      merged.sort((a, b) => a.createdAt.compareTo(b.createdAt));
      _lastKnownMessages = merged;
      state = AsyncData(merged);

      // Auto-prefetch: if deletions/reactions crowded out displayable messages,
      // loop fetchOlder() until we have enough content to fill the screen.
      // Must clear _initInFlight first so fetchOlder() doesn't short-circuit.
      _initInFlight = false;
      await _ensureMinDisplayable(initVersion);
    } catch (e, st) {
      if (!_isCurrentInit(initVersion)) {
        return;
      }
      final fallbackMessages = state.value ?? _lastKnownMessages;
      if (fallbackMessages != null) {
        debugPrint(
          '[ChannelMessagesNotifier] history sync failed for $channelId: $e',
        );
        state = AsyncData(fallbackMessages);
        return;
      }
      state = AsyncError(e, st);
    } finally {
      if (_isCurrentInit(initVersion)) {
        _initInFlight = false;
      }
    }
  }

  void _handleLiveEvent(NostrEvent event) {
    final current = state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
    final merged = _mergeEvent(current, event);
    _lastKnownMessages = merged;
    state = AsyncData(merged);

    // When a membership system event arrives, refresh the channel member list
    // so the @mention autocomplete picks up new members without a restart.
    if (event.kind == EventKind.systemMessage &&
        _isMembershipEvent(event.content)) {
      ref.invalidate(channelMembersProvider(channelId));
    }
  }

  static bool _isMembershipEvent(String content) {
    return content.contains('member_joined') ||
        content.contains('member_left') ||
        content.contains('member_removed');
  }

  /// reactions, edits, legacy pre-migration messages, task markers).
  static const _metadataKinds = {
    EventKind.deletion,
    EventKind.reaction,
    40001,
    EventKind.streamMessageEdit,
    EventKind.huddleParticipantJoined,
    EventKind.huddleParticipantLeft,
    EventKind.agentConversationCompat,
    EventKind.agentConversation,
  };

  /// Minimum displayable messages we want after the initial history load.
  static const _minDisplayable = 15;

  /// Max extra fetchOlder rounds during auto-prefetch to avoid hammering the
  /// relay.
  static const _maxPrefetchRounds = 3;

  /// After the initial history fetch, check whether enough user-visible
  /// messages were loaded. If deletion/reaction events consumed most of the
  /// fetch limit, loop [fetchOlder] to backfill displayable content.
  Future<void> _ensureMinDisplayable(int initVersion) async {
    for (var i = 0; i < _maxPrefetchRounds; i++) {
      if (!_isCurrentInit(initVersion) || _reachedOldest) return;

      final events = state.value;
      if (events == null) return;

      final displayable = events
          .where((e) => !_metadataKinds.contains(e.kind))
          .length;
      if (displayable >= _minDisplayable) return;

      final loaded = await fetchOlder();
      if (!loaded) return;
    }
  }

  /// Merge a new event into the sorted list, deduplicating by ID.
  static List<NostrEvent> _mergeEvent(
    List<NostrEvent> current,
    NostrEvent incoming,
  ) {
    if (current.any((e) => e.id == incoming.id)) return current;
    final updated = [...current, incoming];
    updated.sort((a, b) => a.createdAt.compareTo(b.createdAt));
    return updated;
  }

  bool _isCurrentInit(int initVersion) => initVersion == _initVersion;

  void _clearSubscription() {
    _unsubscribe?.call();
    _unsubscribe = null;
  }

  /// Whether all history has been loaded (no more older messages).
  bool get reachedOldest => _reachedOldest;

  /// Fetch older messages (pagination). Call this when the user scrolls up.
  /// Returns `true` if new messages were loaded.
  Future<bool> fetchOlder() async {
    if (_reachedOldest || _initInFlight) return false;

    final currentEvents = state.value;
    if (currentEvents == null || currentEvents.isEmpty) return false;

    final oldest = currentEvents.first.createdAt;
    final session = ref.read(relaySessionProvider.notifier);

    final older = await session.fetchHistory(
      NostrFilter(
        kinds: EventKind.channelEventKinds,
        tags: {
          '#h': [channelId],
        },
        limit: 100,
        until: oldest,
      ),
    );

    if (older.isEmpty) {
      _reachedOldest = true;
      return false;
    }

    // Dedup against existing events. If nothing new remains after dedup
    // (e.g. all returned events share the boundary timestamp), mark as
    // exhausted to avoid an infinite fetch loop.
    final currentIds = state.value?.map((e) => e.id).toSet() ?? {};
    final deduped = older.where((e) => !currentIds.contains(e.id)).toList();

    if (deduped.isEmpty) {
      _reachedOldest = true;
      return false;
    }

    state = state.whenData((events) {
      final merged = [...deduped, ...events];
      merged.sort((a, b) => a.createdAt.compareTo(b.createdAt));
      _lastKnownMessages = merged;
      return merged;
    });
    return true;
  }
}

final channelMessagesProvider =
    NotifierProvider.family<
      ChannelMessagesNotifier,
      AsyncValue<List<NostrEvent>>,
      String
    >(ChannelMessagesNotifier.new);
