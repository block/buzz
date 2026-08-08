import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:nostr/nostr.dart' as nostr;
import 'package:web_socket_channel/web_socket_channel.dart';

import 'huddle_wire.dart';

class HuddlePeer {
  const HuddlePeer({required this.pubkey, required this.peerIndex});
  final String pubkey;
  final int peerIndex;
}

sealed class HuddleTransportEvent {
  const HuddleTransportEvent();
}

class HuddleRosterEvent extends HuddleTransportEvent {
  const HuddleRosterEvent(this.peers);
  final List<HuddlePeer> peers;
}

class HuddlePeerJoinedEvent extends HuddleTransportEvent {
  const HuddlePeerJoinedEvent(this.peer);
  final HuddlePeer peer;
}

class HuddlePeerLeftEvent extends HuddleTransportEvent {
  const HuddlePeerLeftEvent(this.peerIndex);
  final int peerIndex;
}

class HuddleAudioFrameEvent extends HuddleTransportEvent {
  const HuddleAudioFrameEvent({required this.peerIndex, required this.bytes});
  final int peerIndex;
  final Uint8List bytes;
}

class HuddleTransportException implements Exception {
  const HuddleTransportException(this.message);
  final String message;
  @override
  String toString() => message;
}

abstract interface class HuddleTransport {
  Stream<HuddleTransportEvent> get events;
  Future<void> connect({
    required Uri relayWebSocket,
    required String channelId,
    required String? parentChannelId,
    required String nsec,
  });
  void sendAudio(Uint8List bytes);
  Future<void> close({bool sendLeave = true});
}

class WebSocketHuddleTransport implements HuddleTransport {
  final _events = StreamController<HuddleTransportEvent>.broadcast();
  WebSocketChannel? _socket;
  StreamSubscription<Object?>? _subscription;
  Completer<void>? _joined;
  bool _closing = false;

  @override
  Stream<HuddleTransportEvent> get events => _events.stream;

  @override
  Future<void> connect({
    required Uri relayWebSocket,
    required String channelId,
    required String? parentChannelId,
    required String nsec,
  }) async {
    await close(sendLeave: false);
    _closing = false;
    final uri = relayWebSocket.replace(
      path:
          '${relayWebSocket.path.replaceFirst(RegExp(r'/$'), '')}/huddle/$channelId/audio',
    );
    final socket = WebSocketChannel.connect(uri);
    _socket = socket;
    _joined = Completer<void>();
    _subscription = socket.stream.listen(
      (message) => _onMessage(message, relayWebSocket, parentChannelId, nsec),
      onError: (Object error, StackTrace stack) {
        if (!(_joined?.isCompleted ?? true)) {
          _joined?.completeError(error, stack);
        }
        _events.addError(error, stack);
      },
      onDone: () {
        if (_closing) return;
        final error = const HuddleTransportException(
          'Huddle connection closed',
        );
        if (!(_joined?.isCompleted ?? true)) _joined?.completeError(error);
        _events.addError(error);
      },
    );
    await socket.ready.timeout(const Duration(seconds: 5));
    await _joined!.future.timeout(const Duration(seconds: 5));
  }

  void _onMessage(
    Object? message,
    Uri relayWebSocket,
    String? parentChannelId,
    String nsec,
  ) {
    if (message is List<int>) {
      final bytes = Uint8List.fromList(message);
      if (bytes.length < huddleHeaderLength + 2 ||
          bytes.length > huddleMaxFrameBytes + 1) {
        return;
      }
      _events.add(
        HuddleAudioFrameEvent(
          peerIndex: bytes.first,
          bytes: Uint8List.sublistView(bytes, 1),
        ),
      );
      return;
    }
    if (message is! String || message.length > 8192) return;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } catch (_) {
      _events.addError(
        const HuddleTransportException('Malformed Huddle control message'),
      );
      return;
    }
    if (decoded is! Map<String, dynamic>) return;
    switch (decoded['type']) {
      case 'challenge':
        final challenge = decoded['challenge'];
        if (challenge is! String || challenge.isEmpty) {
          _events.addError(
            const HuddleTransportException('Missing Huddle challenge'),
          );
          return;
        }
        _socket?.sink.add(
          buildHuddleAuthMessage(
            relayWebSocket: relayWebSocket,
            challenge: challenge,
            parentChannelId: parentChannelId,
            nsec: nsec,
          ),
        );
      case 'joined':
        final peers = _parsePeers(decoded['peers']);
        if (!(_joined?.isCompleted ?? true)) _joined?.complete();
        _events.add(HuddleRosterEvent(peers));
      case 'left':
        final index = decoded['peer_index'];
        if (index is int) _events.add(HuddlePeerLeftEvent(index));
      case 'error':
        final error = HuddleTransportException(
          decoded['message']?.toString() ?? 'Huddle relay error',
        );
        if (!(_joined?.isCompleted ?? true)) _joined?.completeError(error);
        _events.addError(error);
    }
  }

  List<HuddlePeer> _parsePeers(Object? raw) {
    if (raw is! List) return const [];
    return raw
        .take(256)
        .whereType<Map>()
        .map((peer) {
          final pubkey = peer['pubkey'];
          final index = peer['peer_index'];
          if (pubkey is! String ||
              pubkey.length != 64 ||
              !RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(pubkey) ||
              index is! int ||
              index < 0 ||
              index > 255) {
            return null;
          }
          return HuddlePeer(pubkey: pubkey, peerIndex: index);
        })
        .whereType<HuddlePeer>()
        .toList();
  }

  @override
  void sendAudio(Uint8List bytes) {
    if (bytes.length > huddleHeaderLength &&
        bytes.length <= huddleMaxFrameBytes) {
      _socket?.sink.add(bytes);
    }
  }

  @override
  Future<void> close({bool sendLeave = true}) async {
    _closing = true;
    final socket = _socket;
    _socket = null;
    if (socket != null) {
      if (sendLeave) socket.sink.add(jsonEncode({'type': 'leave'}));
      await socket.sink.close();
    }
    await _subscription?.cancel();
    _subscription = null;
  }
}

String buildHuddleAuthMessage({
  required Uri relayWebSocket,
  required String challenge,
  required String? parentChannelId,
  required String nsec,
}) {
  final privateKey = nostr.Nip19.decode(payload: nsec).data;
  final event = nostr.Event.from(
    kind: 22242,
    content: '',
    tags: [
      ['relay', relayWebSocket.toString()],
      ['challenge', challenge],
    ],
    secretKey: privateKey,
    verify: false,
  );
  return jsonEncode({
    'type': 'auth',
    'event': event.toMap(),
    'parent_channel_id': parentChannelId,
    'protocol_version': huddleProtocolVersion,
  });
}
