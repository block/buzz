import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../auth/auth.dart';
import 'nostr_models.dart';
import 'relay_provider.dart';
import 'relay_socket.dart';

enum SessionStatus { disconnected, connecting, connected, reconnecting }

@immutable
class SessionState {
  final SessionStatus status;
  final int reconnectAttempt;

  const SessionState({required this.status, this.reconnectAttempt = 0});
}

class _HistorySubscription {
  final List<NostrEvent> events = [];
  final Completer<List<NostrEvent>> completer;
  final Timer timeout;

  _HistorySubscription({required this.completer, required this.timeout});
}

class _LiveSubscription {
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;
  Completer<void>? readyCompleter;
  int? lastSeenCreatedAt;

  _LiveSubscription({
    required this.filter,
    required this.onEvent,
    this.onClosed,
    this.readyCompleter,
  });
}

class _PendingEvent {
  final Completer<NostrEvent> completer;
  final Timer timeout;

  _PendingEvent({required this.completer, required this.timeout});
}

class _BufferedEvent {
  final String subId;
  final NostrEvent event;

  _BufferedEvent(this.subId, this.event);
}

/// Manages websocket subscriptions, event batching, reconnection with replay,
/// and pending event tracking. Equivalent to the desktop's RelayClientSession.
class RelaySessionNotifier extends Notifier<SessionState> {
  static const _baseReconnectDelayMs = 1000;
  static const _maxReconnectDelayMs = 30000;
  static const _eventBatchMs = 16;
  static const _reconnectReplaySkewSeconds = 5;
  static const _maxRecentDeliveryKeys = 5000;

  RelaySocket? _socket;
  final Map<String, _HistorySubscription> _historySubscriptions = {};
  final Map<String, _LiveSubscription> _liveSubscriptions = {};
  final Map<String, _PendingEvent> _pendingEvents = {};
  final List<_BufferedEvent> _eventBuffer = [];
  final Set<String> _recentDeliveryKeys = {};
  Timer? _reconnectTimer;
  Timer? _flushTimer;
  Timer? _backgroundGraceTimer;
  int _reconnectDelayMs = _baseReconnectDelayMs;
  int _subIdCounter = 0;
  bool _disposed = false;

  @override
  SessionState build() {
    final config = ref.watch(relayConfigProvider);
    final authState = ref.watch(authProvider);

    // Reset disposed flag — build() may re-run on the same Notifier instance
    // after a provider dependency changes (e.g. auth completing).
    _disposed = false;

    ref.onDispose(_dispose);

    // Auto-connect when authenticated and we have a signing key (NIP-42 AUTH).
    final isAuthenticated = authState.value?.status == AuthStatus.authenticated;
    if (isAuthenticated && config.nsec != null) {
      // Schedule connection after build completes.
      Future.microtask(() => _connect(config));
    }

    return const SessionState(status: SessionStatus.disconnected);
  }

  /// Fetch historical events matching [filter]. Sends REQ, collects events
  /// until EOSE, then resolves. One-shot subscription.
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    final subId = _nextSubId('h');
    final completer = Completer<List<NostrEvent>>();

    final timer = Timer(timeout, () {
      final sub = _historySubscriptions.remove(subId);
      if (sub != null && !sub.completer.isCompleted) {
        // Resolve with whatever we collected so far rather than failing.
        sub.completer.complete(sub.events);
      }
      _sendClose(subId);
    });

    _historySubscriptions[subId] = _HistorySubscription(
      completer: completer,
      timeout: timer,
    );

    _sendReq(subId, filter);
    return completer.future;
  }

  /// Subscribe to live events matching [filter]. Returns an unsubscribe
  /// function. Live subscriptions survive reconnects — they are replayed with
  /// `since: lastSeenCreatedAt - 5s` on reconnect.
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    final subId = _nextSubId('l');
    final readyCompleter = Completer<void>();

    _liveSubscriptions[subId] = _LiveSubscription(
      filter: filter,
      onEvent: onEvent,
      onClosed: onClosed,
      readyCompleter: readyCompleter,
    );

    _sendReq(subId, filter);

    // Wait for EOSE or a short fallback timeout.
    try {
      await readyCompleter.future.timeout(
        const Duration(milliseconds: 500),
        onTimeout: () {},
      );
    } catch (_) {
      _liveSubscriptions.remove(subId);
      _recentDeliveryKeys.removeWhere((key) => key.startsWith('$subId:'));
      rethrow;
    }
    final liveSub = _liveSubscriptions[subId];
    if (liveSub != null && liveSub.readyCompleter == readyCompleter) {
      liveSub.readyCompleter = null;
    }

    return () => _unsubscribe(subId);
  }

  /// Publish an event and wait for the relay's OK confirmation.
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    final completer = Completer<NostrEvent>();

    final timer = Timer(timeout, () {
      final pending = _pendingEvents.remove(event.id);
      if (pending != null && !pending.completer.isCompleted) {
        pending.completer.completeError(
          TimeoutException(
            'Event ${event.id} not acknowledged within $timeout',
          ),
        );
      }
    });

    _pendingEvents[event.id] = _PendingEvent(
      completer: completer,
      timeout: timer,
    );

    _socket?.send(['EVENT', event.toJson()]);
    return completer.future;
  }

  /// Send a raw message over the WebSocket without waiting for acknowledgement.
  /// Used for ephemeral events like typing indicators.
  void sendRaw(List<dynamic> payload) {
    _socket?.send(payload);
  }

  @visibleForTesting
  void debugHandleMessage(List<dynamic> data) => _handleMessage(data);

  @visibleForTesting
  void debugFlushEventBuffer() => _flushEventBuffer();

  /// Force a reconnect (e.g., returning from background).
  Future<void> reconnect() async {
    await _socket?.disconnect();
    _reconnectDelayMs = _baseReconnectDelayMs;
    final config = ref.read(relayConfigProvider);
    await _connect(config);
  }

  /// Called by the app lifecycle provider when the app goes to background.
  void onAppPaused() {
    _backgroundGraceTimer?.cancel();
    _backgroundGraceTimer = Timer(const Duration(seconds: 5), () {
      _socket?.disconnect();
      state = const SessionState(status: SessionStatus.disconnected);
    });
  }

  /// Called by the app lifecycle provider when the app returns to foreground.
  void onAppResumed() {
    _backgroundGraceTimer?.cancel();
    _backgroundGraceTimer = null;

    // If still connected, nothing to do — the socket survived the background
    // grace window.
    if (state.status == SessionStatus.connected) return;

    // Cancel any in-flight reconnect backoff timer so we reconnect immediately
    // instead of waiting for the (possibly large) exponential delay.
    _reconnectTimer?.cancel();
    _reconnectDelayMs = _baseReconnectDelayMs;
    final config = ref.read(relayConfigProvider);
    _connect(config);
  }

  Future<void> _connect(RelayConfig config) async {
    if (_disposed) return;
    if (_socket?.state == SocketState.connecting ||
        _socket?.state == SocketState.authenticating) {
      return;
    }

    state = SessionState(
      status: SessionStatus.connecting,
      reconnectAttempt: state.reconnectAttempt,
    );

    _socket?.dispose();
    _socket = RelaySocket(
      wsUrl: config.wsUrl,
      nsec: config.nsec,
      onMessage: _handleMessage,
      onConnected: _handleConnected,
      onDisconnected: _handleDisconnected,
    );

    await _socket!.connect();
  }

  void _handleConnected() {
    if (_disposed) return;
    _reconnectDelayMs = _baseReconnectDelayMs;
    state = const SessionState(status: SessionStatus.connected);
    _replayLiveSubscriptions();
  }

  void _handleDisconnected(Object? error) {
    if (_disposed) return;
    _cancelAllHistory(error);
    _rejectAllPending(error);
    _eventBuffer.clear();
    _flushTimer?.cancel();
    _flushTimer = null;
    _scheduleReconnect();
  }

  void _scheduleReconnect() {
    if (_disposed) return;
    if (_liveSubscriptions.isEmpty) {
      state = const SessionState(status: SessionStatus.disconnected);
      return;
    }

    final attempt = state.reconnectAttempt + 1;
    state = SessionState(
      status: SessionStatus.reconnecting,
      reconnectAttempt: attempt,
    );

    _reconnectTimer?.cancel();
    _reconnectTimer = Timer(Duration(milliseconds: _reconnectDelayMs), () {
      _reconnectDelayMs = min(_reconnectDelayMs * 2, _maxReconnectDelayMs);
      final config = ref.read(relayConfigProvider);
      _connect(config);
    });
  }

  /// Replay all live subscriptions after a reconnect, with a time skew to
  /// catch events that occurred during the disconnect.
  void _replayLiveSubscriptions() {
    for (final entry in _liveSubscriptions.entries) {
      final sub = entry.value;
      final since = sub.lastSeenCreatedAt != null
          ? sub.lastSeenCreatedAt! - _reconnectReplaySkewSeconds
          : null;
      final filter = since != null
          ? sub.filter.copyWithSince(since)
          : sub.filter;
      _sendReq(entry.key, filter);
    }
  }

  void _handleMessage(List<dynamic> data) {
    if (data.isEmpty) return;
    final type = data[0] as String;

    switch (type) {
      case 'EVENT':
        _handleEvent(data);
      case 'EOSE':
        _handleEose(data);
      case 'CLOSED':
        _handleClosed(data);
      case 'OK':
        _handleOk(data);
    }
  }

  void _handleEvent(List<dynamic> data) {
    if (data.length < 3) return;
    final subId = data[1] as String;
    final eventJson = data[2] as Map<String, dynamic>;
    final event = NostrEvent.fromJson(eventJson);

    // History subscriptions accumulate immediately.
    final historySub = _historySubscriptions[subId];
    if (historySub != null) {
      historySub.events.add(event);
      return;
    }

    // Live subscriptions get batched.
    final liveSub = _liveSubscriptions[subId];
    if (liveSub != null) {
      // Track last seen timestamp for reconnect replay.
      if (liveSub.lastSeenCreatedAt == null ||
          event.createdAt > liveSub.lastSeenCreatedAt!) {
        liveSub.lastSeenCreatedAt = event.createdAt;
      }
      _eventBuffer.add(_BufferedEvent(subId, event));
      _scheduleFlush();
    }
  }

  void _handleEose(List<dynamic> data) {
    if (data.length < 2) return;
    final subId = data[1] as String;

    // History subscription: resolve with collected events.
    final historySub = _historySubscriptions.remove(subId);
    if (historySub != null) {
      historySub.timeout.cancel();
      if (!historySub.completer.isCompleted) {
        historySub.completer.complete(historySub.events);
      }
      _sendClose(subId);
      return;
    }

    // Live subscription: signal ready.
    final liveSub = _liveSubscriptions[subId];
    if (liveSub != null &&
        liveSub.readyCompleter != null &&
        !liveSub.readyCompleter!.isCompleted) {
      liveSub.readyCompleter!.complete();
      liveSub.readyCompleter = null;
    }
  }

  void _handleClosed(List<dynamic> data) {
    if (data.length < 2) return;
    final subId = data[1] as String;
    final message = data.length >= 3 && data[2] is String
        ? data[2] as String
        : 'subscription closed by relay';

    final historySub = _historySubscriptions.remove(subId);
    if (historySub != null) {
      historySub.timeout.cancel();
      if (!historySub.completer.isCompleted) {
        historySub.completer.completeError(Exception(message));
      }
      return;
    }

    final liveSub = _liveSubscriptions.remove(subId);
    if (liveSub == null) return;
    _recentDeliveryKeys.removeWhere((key) => key.startsWith('$subId:'));

    final readyCompleter = liveSub.readyCompleter;
    if (readyCompleter != null && !readyCompleter.isCompleted) {
      readyCompleter.completeError(Exception(message));
      return;
    }

    liveSub.onClosed?.call(message);
  }

  void _handleOk(List<dynamic> data) {
    if (data.length < 3) return;
    final eventId = data[1] as String;
    final accepted = data[2] as bool;
    final message = data.length > 3 && data[3] is String
        ? data[3] as String
        : '';

    final pending = _pendingEvents.remove(eventId);
    if (pending == null) return;
    pending.timeout.cancel();

    if (accepted) {
      // We don't have the full event here; create a minimal placeholder.
      // Command kinds (e.g. 41010, 30620, 46020) return "response:{...}" in
      // the OK message — preserve it in `content` so callers can parse it.
      if (!pending.completer.isCompleted) {
        pending.completer.complete(
          NostrEvent(
            id: eventId,
            pubkey: '',
            createdAt: 0,
            kind: 0,
            tags: [],
            content: message,
            sig: '',
          ),
        );
      }
    } else {
      if (!pending.completer.isCompleted) {
        pending.completer.completeError(
          Exception(message.isNotEmpty ? message : 'Event rejected'),
        );
      }
    }
  }

  void _scheduleFlush() {
    _flushTimer ??= Timer(
      const Duration(milliseconds: _eventBatchMs),
      _flushEventBuffer,
    );
  }

  void _flushEventBuffer() {
    _flushTimer = null;
    if (_eventBuffer.isEmpty) return;

    final batch = List<_BufferedEvent>.from(_eventBuffer);
    _eventBuffer.clear();

    for (final buffered in batch) {
      final sub = _liveSubscriptions[buffered.subId];
      if (sub == null) continue;

      // Deduplicate per subscription. The same relay event can legitimately
      // match multiple live subscriptions, e.g. the channel list unread listener
      // and the open channel message listener.
      final deliveryKey = '${buffered.subId}:${buffered.event.id}';
      if (_recentDeliveryKeys.contains(deliveryKey)) continue;

      // Cap the dedup set to prevent unbounded memory growth.
      if (_recentDeliveryKeys.length >= _maxRecentDeliveryKeys) {
        _recentDeliveryKeys.clear();
      }
      _recentDeliveryKeys.add(deliveryKey);

      sub.onEvent(buffered.event);
    }
  }

  String _nextSubId(String prefix) {
    _subIdCounter++;
    return '$prefix-$_subIdCounter';
  }

  void _sendReq(String subId, NostrFilter filter) {
    _socket?.send(['REQ', subId, filter.toJson()]);
  }

  void _sendClose(String subId) {
    _socket?.send(['CLOSE', subId]);
  }

  void _unsubscribe(String subId) {
    _liveSubscriptions.remove(subId);
    _recentDeliveryKeys.removeWhere((key) => key.startsWith('$subId:'));
    _sendClose(subId);
  }

  void _cancelAllHistory(Object? error) {
    for (final entry in _historySubscriptions.values) {
      entry.timeout.cancel();
      if (!entry.completer.isCompleted) {
        entry.completer.completeError(error ?? Exception('Connection lost'));
      }
    }
    _historySubscriptions.clear();
  }

  void _rejectAllPending(Object? error) {
    for (final entry in _pendingEvents.values) {
      entry.timeout.cancel();
      if (!entry.completer.isCompleted) {
        entry.completer.completeError(error ?? Exception('Connection lost'));
      }
    }
    _pendingEvents.clear();
  }

  void _dispose() {
    _disposed = true;
    _reconnectTimer?.cancel();
    _flushTimer?.cancel();
    _backgroundGraceTimer?.cancel();
    _cancelAllHistory(null);
    _rejectAllPending(null);
    _recentDeliveryKeys.clear();
    _socket?.dispose();
    _socket = null;
  }
}

final relaySessionProvider =
    NotifierProvider<RelaySessionNotifier, SessionState>(
      RelaySessionNotifier.new,
    );
