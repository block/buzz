import 'dart:async';
import 'dart:convert';
import 'dart:io' show HttpClient;

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import 'nostr_models.dart';

/// Low-level websocket connection with NIP-42 authentication.
///
/// Handles the raw websocket lifecycle: connect, authenticate via NIP-42
/// challenge/response, send/receive JSON frames, and disconnect.
///
/// Does NOT handle reconnection — that is [RelaySessionNotifier]'s job.
enum SocketState { disconnected, connecting, authenticating, connected }

class RelayAuthRejectedException implements Exception {
  final String message;

  const RelayAuthRejectedException(this.message);

  @override
  String toString() => 'Relay authentication rejected: $message';
}

Exception classifyRelayAuthFailure(String message) {
  if (message.startsWith('error:')) return Exception(message);
  return RelayAuthRejectedException(message);
}

class RelaySocket {
  /// Interval for sending a ping and awaiting its pong before disconnecting.
  static const pingInterval = Duration(seconds: 30);

  @visibleForTesting
  static Duration debugPingInterval = pingInterval;

  final String _wsUrl;
  final String? _nsec;
  final void Function(List<dynamic> message) _onMessage;
  final void Function() _onConnected;
  final void Function(Object? error) _onDisconnected;

  WebSocketChannel? _channel;
  HttpClient? _connectingClient;
  StreamSubscription<dynamic>? _subscription;
  SocketState _state = SocketState.disconnected;
  Completer<void>? _authCompleter;
  Timer? _authTimeout;
  String? _pendingAuthEventId;

  SocketState get state => _state;

  RelaySocket({
    required String wsUrl,
    required String? nsec,
    required void Function(List<dynamic> message) onMessage,
    required void Function() onConnected,
    required void Function(Object? error) onDisconnected,
  }) : _wsUrl = wsUrl,
       _nsec = nsec,
       _onMessage = onMessage,
       _onConnected = onConnected,
       _onDisconnected = onDisconnected;

  /// Connect to the relay and complete NIP-42 authentication.
  Future<void> connect() async {
    if (_state != SocketState.disconnected) return;
    _state = SocketState.connecting;

    final client = HttpClient();
    _connectingClient = client;
    final WebSocketChannel channel;
    try {
      channel = IOWebSocketChannel.connect(
        Uri.parse(_wsUrl),
        pingInterval: debugPingInterval,
        customClient: client,
      );
      _channel = channel;
      await channel.ready;
    } catch (e) {
      // Explicit disconnect/dispose already retired this attempt. It must not
      // notify the session or overwrite a newer connection's state.
      if (identical(_connectingClient, client)) {
        _channel = null;
        _state = SocketState.disconnected;
        _onDisconnected(e);
      }
      return;
    } finally {
      // Retire this attempt's HTTP transport. A successful upgrade has already
      // detached its socket, so it remains open for authentication.
      client.close(force: true);
      if (identical(_connectingClient, client)) _connectingClient = null;
    }

    if (!identical(_channel, channel)) return;

    _state = SocketState.authenticating;
    _authCompleter = Completer<void>();

    _subscription = _channel!.stream.listen(
      _handleRawMessage,
      onError: (Object error) {
        _failAuth(error);
        _resetConnection();
        _onDisconnected(error);
      },
      onDone: () {
        _failAuth(null);
        _resetConnection();
        _onDisconnected(null);
      },
    );

    // Wait for auth to complete (or timeout).
    _authTimeout = Timer(const Duration(seconds: 8), () {
      if (_authCompleter != null && !_authCompleter!.isCompleted) {
        _authCompleter!.completeError(
          TimeoutException('NIP-42 auth timed out after 8s'),
        );
      }
    });

    try {
      await _authCompleter!.future;
      _authTimeout?.cancel();
      _state = SocketState.connected;
      _onConnected();
    } catch (e) {
      _authTimeout?.cancel();
      await disconnect();
      _onDisconnected(e);
    }
  }

  /// Send a raw JSON array over the websocket.
  void send(List<dynamic> payload) {
    _channel?.sink.add(jsonEncode(payload));
  }

  /// Gracefully close the connection.
  Future<void> disconnect() async {
    final wasConnecting = _state == SocketState.connecting;
    _resetConnection();
    final channel = _channel;
    _channel = null;
    // Before ready, the channel has no WebSocket to close. Its sink's close
    // future may never settle; _resetConnection has closed the HTTP client.
    if (channel != null && !wasConnecting) {
      await channel.sink.close();
    }
  }

  void dispose() {
    final wasConnecting = _state == SocketState.connecting;
    _resetConnection();
    if (!wasConnecting) _channel?.sink.close();
    _channel = null;
  }

  void _resetConnection() {
    final client = _connectingClient;
    _connectingClient = null;
    client?.close(force: true);
    _state = SocketState.disconnected;
    _subscription?.cancel();
    _subscription = null;
    _authTimeout?.cancel();
    _authTimeout = null;
    _pendingAuthEventId = null;
  }

  void _failAuth(Object? error) {
    if (_authCompleter != null && !_authCompleter!.isCompleted) {
      _authCompleter!.completeError(error ?? Exception('Connection closed'));
    }
  }

  void _handleRawMessage(dynamic raw) {
    final String text;
    if (raw is String) {
      text = raw;
    } else {
      return; // Binary frames are not part of the Nostr protocol.
    }

    final List<dynamic> data;
    try {
      data = jsonDecode(text) as List<dynamic>;
    } catch (_) {
      return; // Malformed JSON.
    }

    if (data.isEmpty) return;
    final type = data[0] as String;

    switch (type) {
      case 'AUTH':
        _handleAuthChallenge(data);
      case 'OK':
        _handleOk(data);
      default:
        // Pass EVENT, EOSE, NOTICE, etc. upstream.
        _onMessage(data);
    }
  }

  /// Handle the relay's AUTH challenge: sign a kind:22242 event and respond.
  void _handleAuthChallenge(List<dynamic> data) {
    if (data.length < 2) return;
    final challenge = data[1] as String;

    if (_nsec == null) {
      _failAuth(Exception('No nsec available for NIP-42 auth'));
      return;
    }

    try {
      // Decode bech32 nsec to hex private key.
      final privkeyHex = nostr.Nip19.decode(payload: _nsec).data;
      if (privkeyHex.isEmpty) {
        _failAuth(Exception('Invalid nsec'));
        return;
      }

      // Build the auth tags.
      final tags = <List<String>>[
        ['relay', _wsUrl],
        ['challenge', challenge],
      ];

      // Create and sign the kind:22242 AUTH event.
      final event = nostr.Event.from(
        kind: EventKind.auth,
        content: '',
        tags: tags,
        secretKey: privkeyHex,
      );

      _pendingAuthEventId = event.id;
      send(['AUTH', event.toMap()]);
    } catch (e) {
      _failAuth(e);
    }
  }

  @visibleForTesting
  void debugHandleOkForTest(List<dynamic> data) => _onMessage(data);

  /// Handle OK frames. During auth, complete the auth flow.
  void _handleOk(List<dynamic> data) {
    if (data.length < 3) return;
    final eventId = data[1] as String;
    final accepted = data[2] as bool;

    // Check if this OK is for our pending AUTH event.
    if (_pendingAuthEventId != null && eventId == _pendingAuthEventId) {
      _pendingAuthEventId = null;
      if (accepted) {
        if (_authCompleter != null && !_authCompleter!.isCompleted) {
          _authCompleter!.complete();
        }
      } else {
        final message = data.length > 3
            ? data[3] as String
            : 'Auth rejected by relay';
        _failAuth(classifyRelayAuthFailure(message));
      }
      return;
    }

    // Pass non-auth OK frames upstream for pending event tracking.
    _onMessage(data);
  }
}
