import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:uuid/uuid.dart';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../auth/auth.dart';
import 'nostr_models.dart';
import 'relay_client.dart';
import 'relay_closed_policy.dart';
import 'relay_provider.dart';
import 'relay_rate_limit_gate.dart';
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
  int closedRetryAttempt = 0;
  Timer? closedRetryTimer;

  _LiveSubscription({
    required this.filter,
    required this.onEvent,
    this.onClosed,
    this.readyCompleter,
  });
}

class _ClosedRetry {
  final _LiveSubscription subscription;
  final int generation;

  _ClosedRetry({required this.subscription, required this.generation});
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
typedef RelaySocketFactory =
    RelaySocket Function({
      required String wsUrl,
      required String? nsec,
      required void Function(List<dynamic> message) onMessage,
      required void Function() onConnected,
      required void Function(Object? error) onDisconnected,
    });

class RelaySessionNotifier extends Notifier<SessionState> {
  RelaySessionNotifier({
    http.Client? httpClient,
    RelaySocketFactory socketFactory = RelaySocket.new,
    DateTime Function()? now,
    RelayRateLimitGate? rateLimitGate,
    RelayTimerFactory retryTimerFactory = Timer.new,
    Future<void> Function(Duration) replayDelay = Future.delayed,
  }) : _httpClient = httpClient,
       _socketFactory = socketFactory,
       _now = now ?? DateTime.now,
       _rateLimitGate = rateLimitGate ?? RelayRateLimitGate(),
       _retryTimerFactory = retryTimerFactory,
       _replayDelay = replayDelay;

  final http.Client? _httpClient;
  final RelaySocketFactory _socketFactory;
  final DateTime Function() _now;
  final RelayRateLimitGate _rateLimitGate;
  final RelayTimerFactory _retryTimerFactory;
  final Future<void> Function(Duration) _replayDelay;

  static const _baseReconnectDelayMs = 1000;
  static const _maxReconnectDelayMs = 30000;
  static const _eventBatchMs = 16;
  static const _reconnectReplaySkewSeconds = 5;
  static const _replayBatchSize = 8;
  static const _replayInterBatchDelay = Duration(milliseconds: 50);
  static const _maxRecentDeliveryKeys = 5000;
  static const _backgroundGraceDuration = Duration(seconds: 5);

  RelaySocket? _socket;
  final Map<String, _HistorySubscription> _historySubscriptions = {};
  final Map<String, _LiveSubscription> _liveSubscriptions = {};
  final Map<String, _ClosedRetry> _pendingClosedRetries = {};
  final Map<String, _PendingEvent> _pendingEvents = {};
  final List<_BufferedEvent> _eventBuffer = [];
  final Set<String> _recentDeliveryKeys = {};
  Timer? _reconnectTimer;
  Timer? _flushTimer;
  Timer? _backgroundGraceTimer;
  DateTime? _backgroundedAt;
  int _reconnectDelayMs = _baseReconnectDelayMs;
  int _subIdCounter = 0;
  bool _disposed = false;
  bool _paused = false;
  bool _hasConnectedOnce = false;
  int _connectionGeneration = 0;
  final Map<Object, String> _visibleChannelsByOwner = {};
  bool _socketConnected = false;
  bool _closedRetryReplayScheduled = false;

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

  /// Execute a one-shot query via the relay's HTTP bridge (`POST /query`).
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final config = ref.read(relayConfigProvider);
    final url = Uri.parse(config.baseUrl).resolve('/query').toString();
    final bodyBytes = utf8.encode(
      jsonEncode(filters.map((filter) => filter.toJson()).toList()),
    );
    final client = _httpClient ?? http.Client();
    final shouldCloseClient = _httpClient == null;
    final response = await client
        .post(
          Uri.parse(url),
          headers: {
            'Authorization': buildNip98AuthHeader(
              method: 'POST',
              url: url,
              bodyBytes: bodyBytes,
              nsec: config.nsec,
            ),
            'Content-Type': 'application/json',
          },
          body: bodyBytes,
        )
        .timeout(timeout)
        .whenComplete(() {
          if (shouldCloseClient) client.close();
        });
    if (response.statusCode < 200 || response.statusCode >= 300) {
      _activateRateLimitGateFromHttpError(response.body);
      throw RelayException(response.statusCode, response.body);
    }
    final decoded = jsonDecode(response.body);
    if (decoded is! List) {
      throw const FormatException('relay returned malformed query response');
    }
    try {
      return [
        for (final eventJson in decoded)
          if (eventJson is Map<String, dynamic>)
            NostrEvent.fromJson(eventJson)
          else
            throw const FormatException('relay returned malformed query event'),
      ];
    } catch (error) {
      if (error is FormatException) rethrow;
      throw FormatException('relay returned malformed query event: $error');
    }
  }

  void _activateRateLimitGateFromHttpError(String body) {
    final dynamic decoded;
    try {
      decoded = jsonDecode(body);
    } on FormatException {
      return;
    }
    if (decoded is! Map<String, dynamic>) return;
    final message = decoded['error'];
    if (message is! String ||
        classifyRelayClosed(message) != RelayClosedClass.rateLimited) {
      return;
    }
    _rateLimitGate.activate(parseRateLimitRetrySeconds(message));
  }

  /// Fetch historical events matching [filter]. Sends REQ, collects events
  /// until EOSE, then resolves. One-shot subscription.
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (_rateLimitGate.isActive) await _rateLimitGate.wait();
    if (_disposed) throw StateError('Relay session is disposed');
    final subId = _nextSubId('h');
    final completer = Completer<List<NostrEvent>>();

    final timer = Timer(timeout, () {
      final sub = _historySubscriptions.remove(subId);
      if (sub != null && !sub.completer.isCompleted) {
        sub.completer.completeError(
          TimeoutException('Relay history request timed out after $timeout'),
        );
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
    if (_disposed) throw StateError('Relay session is disposed');
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

  @visibleForTesting
  Future<void> debugHandleConnected() =>
      _handleConnected(_connectionGeneration);

  @visibleForTesting
  Future<void> debugReplayLiveSubscriptions() =>
      _replayLiveSubscriptions(_connectionGeneration);

  @visibleForTesting
  void debugDispose() => _dispose();

  @visibleForTesting
  void debugSupersedeConnection() => _connectionGeneration++;

  @visibleForTesting
  void debugHandleDisconnected([Object? error]) {
    _socketConnected = false;
    _handleDisconnected(_connectionGeneration, error);
  }

  @visibleForTesting
  void debugResetClosedRetriesForDisconnect() {
    _socketConnected = false;
    _resetAllClosedRetries();
  }

  @visibleForTesting
  void debugSetSessionStatus(SessionStatus status) {
    _socketConnected = status == SessionStatus.connected;
  }

  @visibleForTesting
  void debugPauseNow() => _pauseNow();

  @visibleForTesting
  void debugHandleSocketMessageForTest(List<dynamic> data) =>
      _handleMessage(data);

  @visibleForTesting
  void debugAttachSocketForTest(RelaySocket socket) {
    _socket?.dispose();
    _socket = socket;
    _socketConnected = true;
  }

  /// Registers a visible channel and returns an owner-scoped release callback.
  /// The most recently registered owner is prioritized during reconnect replay.
  void Function() registerVisibleChannel(String channelId) {
    final owner = Object();
    _visibleChannelsByOwner[owner] = channelId;
    return () => _visibleChannelsByOwner.remove(owner);
  }

  /// Force a reconnect (e.g., returning from background).
  Future<void> reconnect() async {
    _socketConnected = false;
    await _socket?.disconnect();
    _reconnectDelayMs = _baseReconnectDelayMs;
    final config = ref.read(relayConfigProvider);
    await _connect(config);
  }

  /// Called by the app lifecycle provider when the app goes to background.
  void onAppPaused() {
    _backgroundedAt = _now();
    _backgroundGraceTimer?.cancel();
    _backgroundGraceTimer = Timer(_backgroundGraceDuration, _pauseNow);
  }

  void _pauseNow() {
    _paused = true;
    _socketConnected = false;
    _reconnectTimer?.cancel();
    _cancelAllHistory(Exception('App moved to background'));
    _rejectAllPending(Exception('App moved to background'));
    _socket?.disconnect();
    state = const SessionState(status: SessionStatus.disconnected);
  }

  /// Called by the app lifecycle provider when the app returns to foreground.
  void onAppResumed() {
    _paused = false;
    final backgroundedAt = _backgroundedAt;
    _backgroundedAt = null;
    _backgroundGraceTimer?.cancel();
    _backgroundGraceTimer = null;

    final backgroundedLongEnoughToRequireReconnect =
        backgroundedAt != null &&
        _now().difference(backgroundedAt) >= _backgroundGraceDuration;
    if (!backgroundedLongEnoughToRequireReconnect &&
        state.status == SessionStatus.connected) {
      return;
    }

    // Cancel any in-flight reconnect backoff timer so we reconnect immediately
    // instead of waiting for the (possibly large) exponential delay.
    _reconnectTimer?.cancel();
    _reconnectDelayMs = _baseReconnectDelayMs;
    final config = ref.read(relayConfigProvider);
    _connect(config);
  }

  Future<void> _connect(RelayConfig config) async {
    if (_disposed) return;

    final generation = ++_connectionGeneration;
    state = SessionState(
      status: _hasConnectedOnce
          ? SessionStatus.reconnecting
          : SessionStatus.connecting,
      reconnectAttempt: state.reconnectAttempt,
    );

    _socket?.dispose();
    final socket = _socketFactory(
      wsUrl: config.wsUrl,
      nsec: config.nsec,
      onMessage: (message) {
        if (generation == _connectionGeneration) _handleMessage(message);
      },
      onConnected: () => _handleConnected(generation),
      onDisconnected: (error) => _handleDisconnected(generation, error),
    );
    _socket = socket;

    await socket.connect();
  }

  Future<void> _handleConnected(int generation) async {
    if (_disposed || generation != _connectionGeneration) return;
    _socketConnected = true;
    _hasConnectedOnce = true;
    _reconnectDelayMs = _baseReconnectDelayMs;
    state = const SessionState(status: SessionStatus.connected);
    await _replayLiveSubscriptions(generation);
  }

  void _handleDisconnected(int generation, Object? error) {
    if (_disposed || generation != _connectionGeneration) return;
    _socketConnected = false;
    _cancelAllHistory(error);
    _rejectAllPending(error);
    _resetAllClosedRetries();
    _eventBuffer.clear();
    _flushTimer?.cancel();
    _flushTimer = null;
    if (error is RelayAuthRejectedException) {
      _reconnectTimer?.cancel();
      state = const SessionState(status: SessionStatus.disconnected);
      return;
    }
    _scheduleReconnect();
  }

  void _scheduleReconnect() {
    if (_disposed || _paused) return;
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
  Future<void> _replayLiveSubscriptions(int generation) async {
    if (_rateLimitGate.isActive) await _rateLimitGate.wait();
    if (!_isActiveConnection(generation)) return;

    final entries = _liveSubscriptions.entries.toList();
    final visibleChannelId = _visibleChannelsByOwner.isEmpty
        ? null
        : _visibleChannelsByOwner.values.last;
    if (visibleChannelId != null) {
      entries.sort((left, right) {
        final leftVisible =
            left.value.filter.tags['#h']?.contains(visibleChannelId) ?? false;
        final rightVisible =
            right.value.filter.tags['#h']?.contains(visibleChannelId) ?? false;
        if (leftVisible == rightVisible) return 0;
        return leftVisible ? -1 : 1;
      });
    }

    await _sendReplayBatches(entries, generation);
  }

  Future<void> _replayPendingClosedRetries(int generation) async {
    if (!_isActiveConnection(generation)) return;
    final entries = _pendingClosedRetries.entries
        .where((entry) => entry.value.generation == generation)
        .map(
          (entry) => MapEntry<String, _LiveSubscription>(
            entry.key,
            entry.value.subscription,
          ),
        )
        .toList();
    await _sendReplayBatches(entries, generation, pendingClosedRetries: true);
  }

  Future<void> _sendReplayBatches(
    List<MapEntry<String, _LiveSubscription>> entries,
    int generation, {
    bool pendingClosedRetries = false,
  }) async {
    for (var i = 0; i < entries.length; i += _replayBatchSize) {
      if (_rateLimitGate.isActive) await _rateLimitGate.wait();
      if (!_isActiveConnection(generation)) return;
      final batch = entries.sublist(
        i,
        min(i + _replayBatchSize, entries.length),
      );
      for (final entry in batch) {
        if (_liveSubscriptions[entry.key] != entry.value) continue;
        if (pendingClosedRetries) {
          final pendingRetry = _pendingClosedRetries[entry.key];
          if (pendingRetry?.subscription != entry.value ||
              pendingRetry?.generation != generation) {
            continue;
          }
          _pendingClosedRetries.remove(entry.key);
        }
        _sendReq(entry.key, _replayFilter(entry.value));
      }
      if (i + _replayBatchSize < entries.length) {
        await _replayDelay(_replayInterBatchDelay);
      }
    }
  }

  bool _isActiveConnection(int generation) =>
      !_disposed && generation == _connectionGeneration;

  NostrFilter _replayFilter(_LiveSubscription subscription) {
    final since = subscription.lastSeenCreatedAt;
    return since == null
        ? subscription.filter
        : subscription.filter.copyWithSince(
            max(0, since - _reconnectReplaySkewSeconds),
          );
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
      _resetClosedRetry(liveSub);
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
    if (liveSub != null) {
      _resetClosedRetry(liveSub);
    }
    if (liveSub != null &&
        liveSub.readyCompleter != null &&
        !liveSub.readyCompleter!.isCompleted) {
      // EOSE is the boundary between replay and live delivery. Flush any
      // replay events before resolving subscribe(), so callers that begin a
      // one-shot query immediately afterwards cannot classify a delayed batch
      // callback as having arrived during that query.
      _flushBufferedEventsNow();
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
    final closedClass = classifyRelayClosed(message);

    final historySub = _historySubscriptions.remove(subId);
    if (historySub != null) {
      if (closedClass == RelayClosedClass.rateLimited) {
        _rateLimitGate.activate(parseRateLimitRetrySeconds(message));
      }
      historySub.timeout.cancel();
      if (!historySub.completer.isCompleted) {
        historySub.completer.completeError(Exception(message));
      }
      return;
    }

    final liveSub = _liveSubscriptions[subId];
    if (liveSub == null) return;
    final readyCompleter = liveSub.readyCompleter;
    if (closedClass == RelayClosedClass.terminal) {
      if (readyCompleter != null && !readyCompleter.isCompleted) {
        readyCompleter.completeError(Exception(message));
      }
      liveSub.onClosed?.call(message);
      _removeLiveSubscription(subId, liveSub);
      return;
    }
    if (readyCompleter != null && !readyCompleter.isCompleted) {
      readyCompleter.complete();
      liveSub.readyCompleter = null;
    }
    if (liveSub.closedRetryTimer != null) return;

    final attempt = liveSub.closedRetryAttempt;
    final backoffMs = attempt >= 5
        ? _maxReconnectDelayMs
        : _baseReconnectDelayMs * (1 << attempt);
    var delayMs = backoffMs;
    if (closedClass == RelayClosedClass.rateLimited) {
      final retrySeconds = parseRateLimitRetrySeconds(message);
      _rateLimitGate.activate(retrySeconds);
      final fallbackMs =
          (retrySeconds != null && retrySeconds > 0
              ? min(retrySeconds, RelayRateLimitGate.maxRetrySeconds)
              : RelayRateLimitGate.defaultRetrySeconds) *
          1000;
      delayMs = max(
        backoffMs,
        _rateLimitGate.remainingMs() == 0
            ? fallbackMs
            : _rateLimitGate.remainingMs(),
      );
    }

    liveSub.closedRetryAttempt = attempt + 1;
    final retryGeneration = _connectionGeneration;
    liveSub.closedRetryTimer = _retryTimerFactory(
      Duration(milliseconds: delayMs),
      () async {
        liveSub.closedRetryTimer = null;
        if (!_isActiveConnection(retryGeneration) ||
            _liveSubscriptions[subId] != liveSub) {
          return;
        }
        if (_rateLimitGate.isActive) await _rateLimitGate.wait();
        if (!_isActiveConnection(retryGeneration) ||
            _liveSubscriptions[subId] != liveSub ||
            !_socketConnected) {
          return;
        }
        _pendingClosedRetries[subId] = _ClosedRetry(
          subscription: liveSub,
          generation: retryGeneration,
        );
        _scheduleClosedRetryReplay(retryGeneration);
      },
    );
  }

  void _scheduleClosedRetryReplay(int generation) {
    if (_closedRetryReplayScheduled) return;
    _closedRetryReplayScheduled = true;
    scheduleMicrotask(() async {
      try {
        await _replayPendingClosedRetries(generation);
      } finally {
        _closedRetryReplayScheduled = false;
        _pendingClosedRetries.removeWhere(
          (_, retry) => retry.generation != _connectionGeneration,
        );
        if (_pendingClosedRetries.values.any(
          (retry) => retry.generation == _connectionGeneration,
        )) {
          _scheduleClosedRetryReplay(_connectionGeneration);
        }
      }
    });
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

  void _flushBufferedEventsNow() {
    _flushTimer?.cancel();
    _flushTimer = null;
    _flushEventBuffer();
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
    final subscription = _liveSubscriptions[subId];
    if (subscription != null) {
      _removeLiveSubscription(subId, subscription);
    }
    _sendClose(subId);
  }

  void _removeLiveSubscription(String subId, _LiveSubscription subscription) {
    if (_liveSubscriptions[subId] != subscription) return;
    _liveSubscriptions.remove(subId);
    _pendingClosedRetries.remove(subId);
    subscription.closedRetryTimer?.cancel();
    subscription.closedRetryTimer = null;
    _recentDeliveryKeys.removeWhere((key) => key.startsWith('$subId:'));
  }

  void _resetClosedRetry(_LiveSubscription subscription) {
    subscription.closedRetryAttempt = 0;
    subscription.closedRetryTimer?.cancel();
    subscription.closedRetryTimer = null;
  }

  void _cancelAllClosedRetries() {
    _pendingClosedRetries.clear();
    for (final subscription in _liveSubscriptions.values) {
      subscription.closedRetryTimer?.cancel();
      subscription.closedRetryTimer = null;
    }
  }

  void _resetAllClosedRetries() {
    _pendingClosedRetries.clear();
    for (final subscription in _liveSubscriptions.values) {
      _resetClosedRetry(subscription);
    }
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
    _connectionGeneration++;
    _reconnectTimer?.cancel();
    _flushTimer?.cancel();
    _backgroundGraceTimer?.cancel();
    _backgroundedAt = null;
    _cancelAllClosedRetries();
    _rateLimitGate.reset();
    _visibleChannelsByOwner.clear();
    _socketConnected = false;
    _cancelAllHistory(null);
    _rejectAllPending(null);
    final subscriptions = _liveSubscriptions.values.toList();
    _liveSubscriptions.clear();
    for (final subscription in subscriptions) {
      subscription.closedRetryTimer?.cancel();
      subscription.closedRetryTimer = null;
    }
    _recentDeliveryKeys.clear();
    _socket?.dispose();
    _socket = null;
    _httpClient?.close();
  }
}

final relaySessionProvider =
    NotifierProvider<RelaySessionNotifier, SessionState>(
      RelaySessionNotifier.new,
    );

String buildNip98AuthHeader({
  required String method,
  required String url,
  required List<int> bodyBytes,
  required String? nsec,
}) {
  if (nsec == null || nsec.isEmpty) {
    throw Exception('Cannot query relay: no signing key available');
  }
  final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
  if (privkeyHex.isEmpty) {
    throw Exception('Invalid nsec');
  }
  final payloadHash = SHA256Digest()
      .process(Uint8List.fromList(bodyBytes))
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join();
  final event = nostr.Event.from(
    kind: 27235,
    content: '',
    tags: [
      ['u', url],
      ['method', method.toUpperCase()],
      ['payload', payloadHash],
      ['nonce', const Uuid().v4()],
    ],
    secretKey: privkeyHex,
    verify: false,
  );
  return 'Nostr ${base64.encode(utf8.encode(event.toJson()))}';
}
