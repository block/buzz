import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../../../shared/workspace/workspace_provider.dart';
import 'read_state_manager.dart';

class ReadStateState {
  final bool isReady;
  final String? pubkey;
  final Map<String, int> contexts;
  final int version;
  final Set<String> locallyForcedChannelIds;

  const ReadStateState({
    required this.isReady,
    required this.pubkey,
    required this.contexts,
    required this.version,
    this.locallyForcedChannelIds = const {},
  });

  const ReadStateState.inert()
    : isReady = false,
      pubkey = null,
      contexts = const {},
      version = 0,
      locallyForcedChannelIds = const {};

  int? effectiveTimestamp(String contextId) => contexts[contextId];

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
      locallyForcedChannelIds: locallyForcedChannelIds,
    );
  }
}

class ReadStateNotifier extends Notifier<ReadStateState> {
  ReadStateManager? _manager;
  bool _isInitialized = false;
  final Set<String> _locallyForcedChannelIds = {};

  @override
  ReadStateState build() {
    _manager?.dispose(flushPending: false);
    _manager = null;
    _isInitialized = false;
    _locallyForcedChannelIds.clear();

    final relayConfig = ref.watch(relayConfigProvider);
    ref.watch(relaySessionProvider);
    final activeWorkspace = ref.watch(activeWorkspaceProvider).value;

    final nsec = relayConfig.nsec?.trim();
    if (nsec == null || nsec.isEmpty) {
      return const ReadStateState.inert();
    }

    final signedRelay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: nsec,
    );
    final pubkey =
        _normalizePubkey(activeWorkspace?.pubkey) ??
        _safeDerivedPubkey(signedRelay);
    if (pubkey == null) {
      return const ReadStateState.inert();
    }

    final crypto = ReadStateCrypto.tryCreate(nsec: nsec, pubkey: pubkey);
    if (crypto == null) {
      return const ReadStateState.inert();
    }

    final prefs = ref.read(savedPrefsProvider);
    late final ReadStateManager manager;
    manager = ReadStateManager(
      pubkey: pubkey,
      prefs: prefs,
      crypto: crypto,
      relaySession: ref.read(relaySessionProvider.notifier),
      signedEventRelay: signedRelay,
      remoteEnabled: true,
      onChanged: () => _emitManagerState(manager),
    );
    _manager = manager;

    ref.onDispose(() {
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

  void markContextRead(String contextId, int unixTimestamp) {
    _locallyForcedChannelIds.remove(contextId);
    _manager?.markContextRead(contextId, unixTimestamp);
  }

  void markContextUnread(String contextId) {
    final manager = _manager;
    if (manager == null) return;
    _locallyForcedChannelIds.add(contextId);
    state = _stateFromManager(
      manager,
      isReady: _isInitialized,
      previousVersion: state.version,
    );
  }

  void seedContextRead(String contextId, int unixTimestamp) {
    _manager?.seedContextRead(contextId, unixTimestamp);
  }

  void _emitManagerState(ReadStateManager manager) {
    if (_manager != manager) return;
    final advances = manager.drainSyncedAdvances();
    _locallyForcedChannelIds.removeAll(advances);
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
      contexts: manager.effectiveContexts,
      version: (previousVersion ?? 0) + 1,
      locallyForcedChannelIds: Set.unmodifiable(
        Set<String>.from(_locallyForcedChannelIds),
      ),
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

String? _safeDerivedPubkey(SignedEventRelay relay) {
  try {
    return _normalizePubkey(relay.pubkey);
  } catch (e) {
    debugPrint('[ReadStateManager] pubkey derivation failed: $e');
    return null;
  }
}
