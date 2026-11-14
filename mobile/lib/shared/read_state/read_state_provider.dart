import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import '../theme/theme_provider.dart';
import '../community/community_provider.dart';
import 'read_state_manager.dart';
import 'read_state_format.dart';
import 'read_state_time.dart';
import 'thread_follows_format.dart';

class ReadStateState {
  final bool isReady;
  final String? pubkey;
  final Map<String, int> contexts;
  final int version;
  final int retentionFloor;

  /// Forced-unread contexts, each mapped to the channel it belongs to so
  /// channel tiles and badges can surface message-level forces: channels
  /// with a live synced manual unread (keyed by the channel id), and this
  /// device's own `msg:` forces.
  final Map<String, String> forcedUnreadContexts;

  /// Merged synced thread follow choices, by `<channel>:<root>` key.
  final Map<String, ThreadFollowChoice> threadFollows;

  /// Whether [threadFollows] may be missing entries.
  final bool threadFollowsIncomplete;

  const ReadStateState({
    required this.isReady,
    required this.pubkey,
    required this.contexts,
    required this.version,
    this.forcedUnreadContexts = const {},
    this.retentionFloor = 0,
    this.threadFollows = const {},
    this.threadFollowsIncomplete = true,
  });

  const ReadStateState.inert()
    : isReady = false,
      pubkey = null,
      contexts = const {},
      version = 0,
      retentionFloor = 0,
      forcedUnreadContexts = const {},
      threadFollows = const {},
      threadFollowsIncomplete = true;

  /// Channels that should surface as unread because of a forced-unread flag —
  /// either forced directly, or containing a forced message.
  Set<String> get locallyForcedChannelIds =>
      forcedUnreadContexts.values.toSet();

  /// Whether this exact context (channel id or `msg:` key) is forced unread.
  bool isForcedUnread(String contextId) =>
      forcedUnreadContexts.containsKey(contextId);

  int? effectiveTimestamp(String contextId) {
    if (pubkey == null || retentionFloor == 0 || isOverrideContext(contextId)) {
      return contexts[contextId];
    }
    final floor = retentionFloor;
    final stored = contexts[contextId];
    return stored == null || stored < floor ? floor : stored;
  }

  ReadStateState copyWithContext(String contextId, int timestamp) {
    final current = contexts[contextId] ?? 0;
    if (timestamp <= current) {
      return this;
    }

    return ReadStateState(
      isReady: isReady,
      pubkey: pubkey,
      contexts: Map.unmodifiable({...contexts, contextId: timestamp}),
      version: version + 1,
      retentionFloor: retentionFloor,
      forcedUnreadContexts: forcedUnreadContexts,
      threadFollows: threadFollows,
      threadFollowsIncomplete: threadFollowsIncomplete,
    );
  }
}

class ReadStateNotifier extends Notifier<ReadStateState> {
  ReadStateManager? _manager;
  bool _isInitialized = false;
  // Device-local `msg:` forces: their channel, their thread root, and the
  // merged frontier covering the message when it was set.
  final Map<String, ({String channelId, String? threadRootId, int baseline})>
  _forcedUnreadContexts = {};

  @override
  ReadStateState build() {
    _manager?.dispose(flushPending: false);
    _manager = null;
    _isInitialized = false;
    _forcedUnreadContexts.clear();

    final relayConfig = ref.watch(relayConfigProvider);
    // Watch the session notifier, not its state: a status change (app resume,
    // reconnect) must not rebuild the manager and drop session-local forced
    // unread flags. The listener below re-syncs on reconnect instead.
    ref.watch(relaySessionProvider.notifier);
    ref.watch(activeCommunityProvider);

    final nsec = relayConfig.nsec?.trim();
    if (nsec == null || nsec.isEmpty) {
      return const ReadStateState.inert();
    }

    final signedRelay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: nsec,
    );
    final pubkey = ref.watch(_readStatePubkeyProvider);
    if (pubkey == null) return const ReadStateState.inert();
    final crypto = ref.watch(readStateCryptoProvider);
    if (crypto == null) {
      return const ReadStateState.inert();
    }

    final prefs = ref.read(savedPrefsProvider);
    late final ReadStateManager manager;
    manager = ReadStateManager(
      pubkey: pubkey,
      relayUrl: relayConfig.baseUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: ref.read(relaySessionProvider.notifier),
      signedEventRelay: signedRelay,
      remoteEnabled: true,
      onChanged: () => _emitManagerState(manager),
    );
    _manager = manager;
    // Expiry is semantic even on an idle screen with no incoming events.
    final floorTimer = Timer.periodic(const Duration(minutes: 1), (_) {
      if (_manager == manager) _emitManagerState(manager);
    });

    ref.onDispose(() {
      floorTimer.cancel();
      manager.dispose();
      if (_manager == manager) {
        _manager = null;
      }
    });

    ref.listen(appLifecycleProvider, (_, next) {
      if (next == AppLifecycleState.paused ||
          next == AppLifecycleState.detached ||
          next == AppLifecycleState.hidden) {
        unawaited(manager.flush());
      }
    });

    ref.listen(relaySessionProvider, (prev, next) {
      if (prev?.status != SessionStatus.connected &&
          next.status == SessionStatus.connected) {
        unawaited(manager.reinitializeRemote());
      }
    });

    Future.microtask(() async {
      await manager.initialize();
      if (_manager != manager) return;
      _isInitialized = true;
      _emitManagerState(manager);
    });

    return _stateFromManager(manager, isReady: false);
  }

  /// Advance a context's read marker. Clears the forced-unread flag for
  /// exactly this context, if any. An explicit channel-level "Mark read"
  /// (channel tile/menu) should pass [clearForcedMessages] so message-level
  /// forces inside the channel are released too; the automatic read on
  /// channel open must not, so a message deliberately marked unread stays
  /// unread until acted on.
  ///
  /// [explicit] marks a user's Mark read: a `msg:` frontier is published
  /// only then, so other devices see it. A channel read also ends that
  /// channel's synced manual unread.
  void markContextRead(
    String contextId,
    int unixTimestamp, {
    bool clearForcedMessages = false,
    bool explicit = false,
  }) {
    var removed = _forcedUnreadContexts.remove(contextId) != null;
    if (clearForcedMessages) {
      final before = _forcedUnreadContexts.length;
      _forcedUnreadContexts.removeWhere(
        (_, force) => force.channelId == contextId,
      );
      removed = removed || _forcedUnreadContexts.length != before;
    }
    _manager?.markContextRead(contextId, unixTimestamp, explicit: explicit);
    if (removed) {
      _refreshForcedState();
    }
  }

  /// Mark a context unread. A channel id is synchronized whole-channel
  /// state, published to the user's other devices. A `msg:` key stays on
  /// this device, until it is read here or the synced frontier covering the
  /// message moves past where it was, on any device. Returns false when the
  /// action failed and nothing changed.
  bool markContextUnread(
    String contextId, {
    required String channelId,
    String? threadRootId,
  }) {
    final manager = _manager;
    if (manager == null) return false;
    if (contextId == channelId) return manager.markChannelUnread(channelId);
    _forcedUnreadContexts[contextId] = (
      channelId: channelId,
      threadRootId: threadRootId,
      baseline: _coveringFrontier(
        manager,
        contextId: contextId,
        channelId: channelId,
        threadRootId: threadRootId,
      ),
    );
    _refreshForcedState();
    return true;
  }

  /// End a manual unread without moving any read marker. Reading to the
  /// bottom of a channel the reader marked unread uses this: it ends the
  /// manual unread but must not read mentions or replies the reader has not
  /// seen.
  void clearForcedUnread(String contextId) {
    if (_forcedUnreadContexts.remove(contextId) != null) {
      _refreshForcedState();
    } else {
      _manager?.clearChannelOverride(contextId);
    }
  }

  /// Record an explicit follow or unfollow; see [ReadStateManager]. Returns
  /// false when it could not be recorded.
  bool setThreadFollow(String channelId, String rootId, bool follow) =>
      _manager?.setThreadFollow(channelId, rootId, follow) ?? false;

  /// See [ReadStateManager.importLegacyFollows].
  Set<String> importLegacyFollows(Map<String, bool> choices) =>
      _manager?.importLegacyFollows(choices) ?? const {};

  /// The newest merged frontier covering a `msg:` context, without the
  /// retention floor: the floor moves with the clock, not with reading.
  static int _coveringFrontier(
    ReadStateManager manager, {
    required String contextId,
    required String channelId,
    required String? threadRootId,
  }) {
    final contexts = manager.effectiveContexts;
    return maxReadAt([
          contexts[channelId],
          contexts[contextId],
          if (threadRootId != null) ...[
            contexts[threadContextKey(threadRootId)],
            contexts[threadActivityContextKey(threadRootId)],
          ],
        ]) ??
        0;
  }

  void _refreshForcedState() {
    final manager = _manager;
    if (manager == null) return;
    state = _stateFromManager(
      manager,
      isReady: _isInitialized,
      previousVersion: state.version,
    );
  }

  void seedContextRead(String contextId, int unixTimestamp) {
    _manager?.seedContextRead(contextId, unixTimestamp);
  }

  /// Seed the first channel-list landing once, durably across restarts.
  void seedContextsRead(Map<String, int> contexts) {
    _manager?.seedInitialContextsRead(contexts);
  }

  void _emitManagerState(ReadStateManager manager) {
    if (_manager != manager) return;
    final advances = manager.drainSyncedAdvances();
    for (final contextId in advances) {
      _forcedUnreadContexts.remove(contextId);
    }
    // A device-local force ends once the merged frontier covering it moves
    // past where it was, whichever device moved it.
    _forcedUnreadContexts.removeWhere(
      (contextId, force) =>
          _coveringFrontier(
            manager,
            contextId: contextId,
            channelId: force.channelId,
            threadRootId: force.threadRootId,
          ) >
          force.baseline,
    );
    state = _stateFromManager(
      manager,
      isReady: _isInitialized,
      previousVersion: state.version,
    );
  }

  ReadStateState _stateFromManager(
    ReadStateManager manager, {
    required bool isReady,
    int? previousVersion,
  }) {
    return ReadStateState(
      isReady: isReady,
      pubkey: manager.pubkey,
      retentionFloor: readStateFloor(currentUnixSeconds()),
      contexts: manager.effectiveContexts,
      version: (previousVersion ?? 0) + 1,
      forcedUnreadContexts: Map.unmodifiable({
        for (final channelId in manager.activeChannelOverrides)
          channelId: channelId,
        for (final MapEntry(key: contextId, value: force)
            in _forcedUnreadContexts.entries)
          contextId: force.channelId,
      }),
      threadFollows: Map.unmodifiable(manager.threadFollows),
      threadFollowsIncomplete: manager.threadFollowsIncomplete,
    );
  }
}

final readStateProvider = NotifierProvider<ReadStateNotifier, ReadStateState>(
  ReadStateNotifier.new,
);

String? _normalizePubkey(String? value) {
  final normalized = value?.trim().toLowerCase();
  if (normalized == null || normalized.isEmpty) {
    return null;
  }
  return normalized;
}

final _readStatePubkeyProvider = Provider<String?>((ref) {
  final active = ref.watch(activeCommunityProvider).value;
  final config = ref.watch(relayConfigProvider);
  return _normalizePubkey(active?.pubkey) ??
      _normalizePubkey(pubkeyFromNsec(config.nsec?.trim()));
});

/// Retains read-state encryption material across connection status changes.
final readStateCryptoProvider = Provider<ReadStateCrypto?>((ref) {
  final config = ref.watch(relayConfigProvider);
  final pubkey = ref.watch(_readStatePubkeyProvider);
  final nsec = config.nsec?.trim();
  if (nsec == null || nsec.isEmpty || pubkey == null) return null;
  return ReadStateCrypto.tryCreate(nsec: nsec, pubkey: pubkey);
});
