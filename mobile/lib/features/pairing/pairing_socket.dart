import 'dart:async';
import 'dart:convert';

import 'package:nostr/nostr.dart' as nostr;
import 'package:web_socket_channel/web_socket_channel.dart';

import '../../shared/relay/nostr_models.dart';

/// Ephemeral WebSocket connection for NIP-AB pairing.
///
/// Uses ephemeral keys for NIP-42 auth (not the stored user keys).
/// Single-use — disposed after the pairing session completes.
class PairingSocket {
  final String _wsUrl;
  final String _ephemeralPrivkey;
  final void Function(List<dynamic> message) _onMessage;
  final void Function(Object? error) _onDisconnected;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _subscription;
  Completer<void>? _authCompleter;
  Timer? _authTimeout;
  String? _pendingAuthEventId;
  bool _connected = false;

  PairingSocket({
    required String wsUrl,
    required String ephemeralPrivkey,
    required void Function(List<dynamic> message) onMessage,
    required void Function(Object? error) onDisconnected,
  }) : _wsUrl = wsUrl,
       _ephemeralPrivkey = ephemeralPrivkey,
       _onMessage = onMessage,
       _onDisconnected = onDisconnected;

  bool get isConnected => _connected;

  /// Connect and authenticate via NIP-42.
  Future<void> connect() async {
    try {
      _channel = WebSocketChannel.connect(Uri.parse(_wsUrl));
      await _channel!.ready;
    } catch (e) {
      _onDisconnected(e);
      return;
    }

    _authCompleter = Completer<void>();

    _subscription = _channel!.stream.listen(
      _handleRawMessage,
      onError: (Object error) {
        _failAuth(error);
        _onDisconnected(error);
      },
      onDone: () {
        _failAuth(null);
        _onDisconnected(null);
      },
    );

    // Wait for auth with 8s timeout.
    _authTimeout = Timer(const Duration(seconds: 8), () {
      if (_authCompleter != null && !_authCompleter!.isCompleted) {
        _authCompleter!.completeError(
          TimeoutException('NIP-42 auth timed out'),
        );
      }
    });

    try {
      await _authCompleter!.future;
      _authTimeout?.cancel();
      _connected = true;
    } catch (e) {
      _authTimeout?.cancel();
      await disconnect();
      _onDisconnected(e);
    }
  }

  /// Send a raw JSON array.
  void send(List<dynamic> payload) {
    _channel?.sink.add(jsonEncode(payload));
  }

  /// Send a subscribe request.
  void subscribe(String subId, int kind, String pubkeyHex) {
    send([
      'REQ',
      subId,
      {
        'kinds': [kind],
        '#p': [pubkeyHex],
      },
    ]);
  }

  /// Publish a Nostr event (already JSON-encoded map).
  void publishEvent(Map<String, dynamic> event) {
    send(['EVENT', event]);
  }

  Future<void> disconnect() async {
    _connected = false;
    _subscription?.cancel();
    _subscription = null;
    _authTimeout?.cancel();
    final channel = _channel;
    _channel = null;
    if (channel != null) {
      await channel.sink.close();
    }
  }

  void dispose() {
    _connected = false;
    _subscription?.cancel();
    _channel?.sink.close();
    _channel = null;
    _authTimeout?.cancel();
  }

  void _failAuth(Object? error) {
    if (_authCompleter != null && !_authCompleter!.isCompleted) {
      _authCompleter!.completeError(error ?? Exception('Connection closed'));
    }
  }

  void _handleRawMessage(dynamic raw) {
    if (raw is! String) return;

    final List<dynamic> data;
    try {
      data = jsonDecode(raw) as List<dynamic>;
    } catch (_) {
      return;
    }

    if (data.isEmpty) return;
    final type = data[0] as String;

    switch (type) {
      case 'AUTH':
        _handleAuthChallenge(data);
      case 'OK':
        _handleOk(data);
      default:
        // Pass EVENT, EOSE, NOTICE upstream.
        _onMessage(data);
    }
  }

  void _handleAuthChallenge(List<dynamic> data) {
    if (data.length < 2) return;
    final challenge = data[1] as String;

    try {
      // Build NIP-42 auth event (kind:22242) with ephemeral keys.
      final tags = <List<String>>[
        ['relay', _wsUrl],
        ['challenge', challenge],
      ];

      final event = nostr.Event.from(
        kind: EventKind.auth,
        content: '',
        tags: tags,
        secretKey: _ephemeralPrivkey,
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
      );

      _pendingAuthEventId = event.id;
      send(['AUTH', event.toMap()]);
    } catch (e) {
      _failAuth(e);
    }
  }

  void _handleOk(List<dynamic> data) {
    if (data.length < 3) return;
    final eventId = data[1] as String;
    final accepted = data[2] as bool;

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
        _failAuth(Exception(message));
      }
      return;
    }

    // Pass non-auth OK upstream.
    _onMessage(data);
  }
}
