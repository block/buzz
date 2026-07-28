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
import 'relay_provider.dart';
import 'relay_socket.dart';

enum SessionStatus { disconnected, connecting, connected, reconnecting }

@immutable
class SessionState {
  final SessionStatus status;
  final int reconnectAttempt;

  const SessionState({required this.status, this.reconnectAttempt = 0});
}

/// A publish was refused because the relay's rate-limit window is still active.
class RelayRateLimitedException implements Exception {
  const RelayRateLimitedException(this.retryAfter);

  final Duration retryAfter;

  @override
  String toString() =>
      'Relay is rate-limited; retry after ${retryAfter.inMilliseconds}ms';
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
    Random? random,
    int Function()? rateLimitNowMs,
  }) : _httpClient = httpClient,
       _socketFactory = socketFactory,
       _random = random ?? Random(),
       _rateLimitClock = Stopwatch()..start(),
       _rateLimitNowMsOverride = rateLimitNowMs;

  final http.Client? _httpClient;
  final RelaySocketFactory _socketFactory;
  final Random _random;
  final Stopwatch _rateLimitClock;
  final int Function()? _rateLimitNowMsOverride;

  static const _baseReconnectDelayMs = 1000;
  static const _maxReconnectDelayMs = 30000;
  static const _eventBatchMs = 16;
  static const _reconnectReplaySkewSeconds = 5;
  static const _maxRecentDeliveryKeys = 5000;

  /// Fraction of the backoff delay randomised in each direction, so a fleet of
  /// clients does not reconnect in lockstep after a relay blip. Mirrors
  /// buzz-acp's `jittered_duration` (crates/buzz-acp/src/relay.rs).
  static const _reconnectJitterRatio = 0.2;

  /// How long a connection must stay up before the backoff ladder is
  /// considered earned back and resets to base. Mirrors buzz-acp's
  /// `STABLE_CONNECTION_SECS` (crates/buzz-acp/src/relay.rs).
  static const _stableConnectionMs = 60000;

  /// Backoff floor applied when the relay rate-limits us without a usable
  /// `retry in Ns` hint (absent, or below 2s). Mirrors the floor in buzz-acp's
  /// `set_rate_limit_gate`.
  static const _minRateLimitBackoffMs = 5000;

  RelaySocket? _socket;
  final Map<String, _HistorySubscription> _historySubscriptions = {};
  final Map<String, _LiveSubscription> _liveSubscriptions = {};
  final Map<String, _PendingEvent> _pendingEvents = {};
  final List<_BufferedEvent> _eventBuffer = [];
  final Set<String> _recentDeliveryKeys = {};
  Timer? _reconnectTimer;
  Timer? _flushTimer;
  Timer? _backgroundGraceTimer;
  Timer? _stableConnectionTimer;
  int _reconnectDelayMs = _baseReconnectDelayMs;
  Duration? _lastReconnectDelay;
  int? _rateLimitDeadlineMs;
  int _subIdCounter = 0;
  bool _disposed = false;
  bool _paused = false;
  bool _hasConnectedOnce = false;
  int _connectionGeneration = 0;

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
    final retryAfter = _checkRateLimitGate();
    if (retryAfter != null) {
      return Future<NostrEvent>.error(RelayRateLimitedException(retryAfter));
    }

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
    if (_checkRateLimitGate() != null) return;
    _socket?.send(payload);
  }

  @visibleForTesting
  void debugHandleMessage(List<dynamic> data) => _handleMessage(data);

  @visibleForTesting
  void debugFlushEventBuffer() => _flushEventBuffer();

  @visibleForTesting
  void debugHandleConnected() => _handleConnected(_connectionGeneration);

  @visibleForTesting
  void debugHandleDisconnected([Object? error]) =>
      _handleDisconnected(_connectionGeneration, error);

  @visibleForTesting
  void debugPauseNow() => _pauseNow();

  @visibleForTesting
  void debugHandleSocketMessageForTest(List<dynamic> data) =>
      _handleMessage(data);

  /// Current position on the reconnect backoff ladder, before jitter.
  @visibleForTesting
  int get debugReconnectDelayMs => _reconnectDelayMs;

  /// The jittered delay most recently handed to the reconnect timer.
  @visibleForTesting
  Duration? get debugLastReconnectDelay => _lastReconnectDelay;

  /// Whether a connection is currently accruing stability toward a backoff
  /// ladder reset.
  @visibleForTesting
  bool get debugStableConnectionArmed => _stableConnectionTimer != null;

  /// Remaining send-side rate-limit delay, or null when the gate is inactive.
  @visibleForTesting
  Duration? get debugRateLimitRemaining => _checkRateLimitGate();

  /// Monotonic deadline used to verify overlapping notices preserve the max.
  @visibleForTesting
  int? get debugRateLimitDeadlineMs {
    if (_checkRateLimitGate() == null) return null;
    return _rateLimitDeadlineMs;
  }

  @visibleForTesting
  Duration debugJitteredDelay(int baseMs) => _jitteredDelay(baseMs);

  /// Run the pending reconnect attempt now instead of waiting out its delay.
  @visibleForTesting
  void debugFireReconnectTimer() {
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    _runScheduledReconnect();
  }

  /// Run the stability reset the connection would reach after
  /// [_stableConnectionMs], without waiting for wall-clock time.
  @visibleForTesting
  void debugCompleteStableConnection() => _onConnectionStable();

  @visibleForTesting
  void debugAttachSocketForTest(RelaySocket socket) {
    _socket?.dispose();
    _socket = socket;
    state = const SessionState(status: SessionStatus.connected);
  }

  /// Force a reconnect (e.g., returning from background).
  Future<void> reconnect() async {
    await _socket?.disconnect();
    // Caller-driven reconnect: reset the ladder immediately. Unlike the
    // automatic reconnect path this is gated by an explicit request, so it
    // cannot spin into a hammering loop on its own.
    _reconnectDelayMs = _baseReconnectDelayMs;
    final config = ref.read(relayConfigProvider);
    await _connect(config);
  }

  /// Called by the app lifecycle provider when the app goes to background.
  void onAppPaused() {
    _backgroundGraceTimer?.cancel();
    _backgroundGraceTimer = Timer(const Duration(seconds: 5), _pauseNow);
  }

  void _pauseNow() {
    _paused = true;
    _reconnectTimer?.cancel();
    _stableConnectionTimer?.cancel();
    _stableConnectionTimer = null;
    _cancelAllHistory(Exception('App moved to background'));
    _rejectAllPending(Exception('App moved to background'));
    _socket?.disconnect();
    state = const SessionState(status: SessionStatus.disconnected);
  }

  /// Called by the app lifecycle provider when the app returns to foreground.
  void onAppResumed() {
    _paused = false;
    _backgroundGraceTimer?.cancel();
    _backgroundGraceTimer = null;

    // If still connected, nothing to do — the socket survived the background
    // grace window.
    if (state.status == SessionStatus.connected) return;

    // Cancel any in-flight reconnect backoff timer so we reconnect immediately
    // instead of waiting for the (possibly large) exponential delay. The
    // preceding disconnect was our own backgrounding, not relay trouble, and
    // the user is looking at the app right now — reset the ladder immediately
    // rather than making them wait out a delay the relay never asked for.
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

  void _handleConnected(int generation) {
    if (_disposed || generation != _connectionGeneration) return;
    _hasConnectedOnce = true;
    // Deliberately does NOT reset the backoff ladder. A socket that dies
    // seconds after connecting would reset it on every cycle, so the doubling
    // never accumulates and a flapping link hammers the relay indefinitely.
    // The ladder is only earned back once the connection proves itself stable.
    _armStableConnectionReset();
    state = const SessionState(status: SessionStatus.connected);
    _replayLiveSubscriptions();
  }

  /// Start counting down to a backoff ladder reset. Cancelled the moment the
  /// connection drops, so only a connection that survives
  /// [_stableConnectionMs] resets the ladder.
  void _armStableConnectionReset() {
    _stableConnectionTimer?.cancel();
    _stableConnectionTimer = Timer(
      const Duration(milliseconds: _stableConnectionMs),
      _onConnectionStable,
    );
  }

  void _onConnectionStable() {
    _stableConnectionTimer?.cancel();
    _stableConnectionTimer = null;
    _reconnectDelayMs = _baseReconnectDelayMs;
  }

  void _handleDisconnected(int generation, Object? error) {
    if (_disposed || generation != _connectionGeneration) return;
    // The connection did not survive long enough to earn the ladder back.
    _stableConnectionTimer?.cancel();
    _stableConnectionTimer = null;
    _cancelAllHistory(error);
    _rejectAllPending(error);
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
    final delay = _jitteredDelay(_reconnectDelayMs);
    _lastReconnectDelay = delay;
    _reconnectTimer = Timer(delay, _runScheduledReconnect);
  }

  void _runScheduledReconnect() {
    _reconnectDelayMs = min(_reconnectDelayMs * 2, _maxReconnectDelayMs);
    final config = ref.read(relayConfigProvider);
    _connect(config);
  }

  /// Randomise [baseMs] by ±[_reconnectJitterRatio] so a fleet of clients does
  /// not reconnect in lockstep after a relay blip. The ladder position itself
  /// stays un-jittered; jitter is applied per scheduled wait.
  Duration _jitteredDelay(int baseMs) {
    final factor =
        1 -
        _reconnectJitterRatio +
        _random.nextDouble() * 2 * _reconnectJitterRatio;
    return Duration(milliseconds: (baseMs * factor).round());
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
      case 'NOTICE':
        _handleNotice(data);
    }
  }

  /// Handle a relay NOTICE. The relay announces rate limiting this way
  /// (`rate-limited: ...; retry in 30s`), so discarding NOTICE means never
  /// hearing the slow-down we are provoking. A rate-limit notice arms a
  /// send-side gate independent of the reconnect backoff ladder.
  void _handleNotice(List<dynamic> data) {
    if (data.length < 2 || data[1] is! String) return;
    final message = data[1] as String;
    debugPrint('relay NOTICE: $message');
    if (!message.startsWith('rate-limited:')) return;

    // A missing hint, or one under 2s, floors to _minRateLimitBackoffMs: a
    // burst of low-quality hints must not drop the gate so short that the next
    // send immediately re-triggers the limit.
    final hintMs = _parseRetryHintMs(message) ?? 0;
    final requested = hintMs < 2000 ? _minRateLimitBackoffMs : hintMs;
    final deadline = _rateLimitNowMs + _jitteredDelay(requested).inMilliseconds;
    // Take the maximum so overlapping notices cannot shorten a later deadline
    // that is already in place. Relay hints are deliberately not clamped by
    // the reconnect ladder's 30-second maximum.
    _rateLimitDeadlineMs = max(_rateLimitDeadlineMs ?? 0, deadline);
  }

  /// Return the remaining gate duration, lazily clearing an expired deadline.
  Duration? _checkRateLimitGate() {
    final deadline = _rateLimitDeadlineMs;
    if (deadline == null) return null;
    final remainingMs = deadline - _rateLimitNowMs;
    if (remainingMs > 0) return Duration(milliseconds: remainingMs);
    _rateLimitDeadlineMs = null;
    return null;
  }

  int get _rateLimitNowMs =>
      _rateLimitNowMsOverride?.call() ?? _rateLimitClock.elapsedMilliseconds;

  /// Parse the relay's `retry in {N}s` hint into milliseconds. Returns null
  /// when the hint is absent or malformed. Mirrors buzz-acp's
  /// `parse_rate_limit_retry_secs`.
  int? _parseRetryHintMs(String message) {
    final match = RegExp(r'retry in (\d+)').firstMatch(message);
    if (match == null) return null;
    final seconds = int.tryParse(match.group(1)!);
    return seconds == null ? null : seconds * 1000;
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
    _connectionGeneration++;
    _reconnectTimer?.cancel();
    _flushTimer?.cancel();
    _backgroundGraceTimer?.cancel();
    _stableConnectionTimer?.cancel();
    _cancelAllHistory(null);
    _rejectAllPending(null);
    _recentDeliveryKeys.clear();
    _rateLimitDeadlineMs = null;
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
