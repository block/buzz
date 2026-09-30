import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import 'huddle_auth.dart';
import 'huddle_wire.dart';

typedef HuddleWebSocketFactory = WebSocketChannel Function(Uri uri);

WebSocketChannel _openHuddleWebSocket(Uri uri) =>
    IOWebSocketChannel.connect(uri, pingInterval: const Duration(seconds: 30));

enum HuddleTransportPhase {
  idle,
  connecting,
  awaitingChallenge,
  authenticating,
  connected,
  closing,
  disconnected,
  failed,
}

enum HuddleTransportErrorCode {
  invalidState,
  connectionFailed,
  handshakeTimeout,
  authenticationFailed,
  relayRejected,
  protocolViolation,
  socketClosed,
  sendFailed,
  cancelled,
  disposed,
}

@immutable
final class HuddleTransportError implements Exception {
  final HuddleTransportErrorCode code;
  final String message;
  final String? relayCode;
  final Object? cause;

  const HuddleTransportError({
    required this.code,
    required this.message,
    this.relayCode,
    this.cause,
  });

  @override
  String toString() {
    final suffix = relayCode == null ? '' : ' ($relayCode)';
    return 'HuddleTransportError.${code.name}$suffix: $message';
  }
}

@immutable
final class HuddlePeer {
  final String pubkey;
  final int peerIndex;

  /// Occupancy epoch of `peerIndex` for this pubkey. Advances each time the
  /// index is reused by another occupancy; media frames are fenced against it
  /// so a prior occupant's in-flight audio cannot be mis-attributed after reuse.
  final int epoch;

  const HuddlePeer({
    required this.pubkey,
    required this.peerIndex,
    required this.epoch,
  });
}

enum HuddlePeerEventType { joined, left, replaced }

@immutable
final class HuddlePeerEvent {
  final HuddlePeerEventType type;
  final HuddlePeer peer;
  final HuddlePeer? replacement;

  const HuddlePeerEvent({
    required this.type,
    required this.peer,
    this.replacement,
  });
}

@immutable
final class HuddleTransportState {
  final HuddleTransportPhase phase;
  final int? localPeerIndex;
  final int? rosterRevision;
  final UnmodifiableMapView<int, HuddlePeer> peers;
  final HuddleTransportError? error;

  HuddleTransportState({
    required this.phase,
    this.localPeerIndex,
    this.rosterRevision,
    Map<int, HuddlePeer> peers = const {},
    this.error,
  }) : peers = UnmodifiableMapView(Map<int, HuddlePeer>.from(peers));

  factory HuddleTransportState.idle() =>
      HuddleTransportState(phase: HuddleTransportPhase.idle);
}

/// Dedicated Huddle audio transport.
///
/// This socket intentionally does not reuse [RelaySocket]: the main Nostr
/// connection speaks JSON arrays and ignores binary messages, while Huddle
/// audio uses JSON objects for its handshake/control plane and binary Opus v2
/// frames for media. [connect] completes only after the relay's `joined`
/// response, making authentication and room admission observable to callers.
abstract interface class HuddleTransportClient {
  HuddleTransportState get state;
  Stream<HuddleTransportState> get states;
  Stream<HuddleRemoteAudioFrame> get remoteAudioFrames;
  Stream<HuddlePeerEvent> get peerEvents;
  Stream<HuddleTransportError> get issues;

  Future<void> connect();
  void sendOpusFrame({
    required HuddleAudioHeader header,
    required Uint8List opusPayload,
  });
  Future<void> disconnect();
  Future<void> dispose();
}

final class HuddleTransport implements HuddleTransportClient {
  final HuddleConnectionParameters parameters;
  final Duration connectTimeout;
  final Duration handshakeTimeout;
  final HuddleWebSocketFactory _channelFactory;

  final StreamController<HuddleTransportState> _stateController =
      StreamController.broadcast(sync: true);
  final StreamController<HuddleRemoteAudioFrame> _audioController =
      StreamController.broadcast(sync: true);
  static const _audioIngressCapacity = 50;
  final Queue<HuddleRemoteAudioFrame> _audioIngress = Queue();
  Timer? _audioIngressTimer;
  final StreamController<HuddlePeerEvent> _peerController =
      StreamController.broadcast(sync: true);
  final StreamController<HuddleTransportError> _issueController =
      StreamController.broadcast(sync: true);

  HuddleTransportState _state = HuddleTransportState.idle();
  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _subscription;
  Completer<void>? _handshakeCompleter;
  Timer? _handshakeTimer;
  int _generation = 0;
  bool _disposed = false;
  bool _intentionalClose = false;
  bool _hasAdmitted = false;
  Map<int, HuddlePeer>? _admissionPreviousPeers;

  HuddleTransport({
    required this.parameters,
    this.connectTimeout = const Duration(seconds: 8),
    this.handshakeTimeout = const Duration(seconds: 5),
    HuddleWebSocketFactory channelFactory = _openHuddleWebSocket,
  }) : _channelFactory = channelFactory;

  @override
  HuddleTransportState get state => _state;
  @override
  Stream<HuddleTransportState> get states => _stateController.stream;
  @override
  Stream<HuddleRemoteAudioFrame> get remoteAudioFrames =>
      _audioController.stream;
  @override
  Stream<HuddlePeerEvent> get peerEvents => _peerController.stream;

  /// Recoverable packet/control problems that do not tear down a joined room.
  @override
  Stream<HuddleTransportError> get issues => _issueController.stream;

  @override
  Future<void> connect() async {
    if (_disposed) {
      throw const HuddleTransportError(
        code: HuddleTransportErrorCode.disposed,
        message: 'Huddle transport has been disposed.',
      );
    }
    if (_state.phase != HuddleTransportPhase.idle &&
        _state.phase != HuddleTransportPhase.disconnected &&
        _state.phase != HuddleTransportPhase.failed) {
      throw HuddleTransportError(
        code: HuddleTransportErrorCode.invalidState,
        message: 'Cannot connect while transport is ${_state.phase.name}.',
      );
    }

    if (_state.phase == HuddleTransportPhase.failed ||
        _state.phase == HuddleTransportPhase.disconnected) {
      _handshakeTimer?.cancel();
      _handshakeTimer = null;
      final previousSubscription = _subscription;
      _subscription = null;
      await previousSubscription?.cancel();
      final previousChannel = _channel;
      _channel = null;
      await previousChannel?.sink.close();
      _handshakeCompleter = null;
    }

    final generation = ++_generation;
    _intentionalClose = false;
    _admissionPreviousPeers = Map<int, HuddlePeer>.from(_state.peers);
    _emitState(
      HuddleTransportState(
        phase: HuddleTransportPhase.connecting,
        localPeerIndex: _state.localPeerIndex,
        // Roster revisions are scoped to one relay process/room lifetime. A
        // fresh socket admission establishes a new authoritative baseline.
        rosterRevision: null,
        peers: _state.peers,
      ),
    );

    try {
      final channel = _channelFactory(parameters.audioWebSocketUri);
      _channel = channel;
      await channel.ready.timeout(connectTimeout);
      if (!_isCurrent(generation)) {
        throw const HuddleTransportError(
          code: HuddleTransportErrorCode.cancelled,
          message: 'Huddle connection was superseded.',
        );
      }

      _handshakeCompleter = Completer<void>();
      _subscription = channel.stream.listen(
        (raw) => _handleRawMessage(raw, generation),
        onError: (Object error, StackTrace stackTrace) =>
            _handleSocketFailure(error, generation),
        onDone: () => _handleSocketClosed(generation),
        cancelOnError: false,
      );
      _emitState(
        HuddleTransportState(
          phase: HuddleTransportPhase.awaitingChallenge,
          localPeerIndex: _state.localPeerIndex,
          rosterRevision: _state.rosterRevision,
          peers: _state.peers,
        ),
      );
      _handshakeTimer = Timer(handshakeTimeout, () {
        _fail(
          const HuddleTransportError(
            code: HuddleTransportErrorCode.handshakeTimeout,
            message: 'Timed out waiting for Huddle room admission.',
          ),
          generation,
        );
      });

      await _handshakeCompleter!.future;
    } on HuddleTransportError {
      rethrow;
    } catch (error) {
      final failure = HuddleTransportError(
        code: HuddleTransportErrorCode.connectionFailed,
        message: 'Unable to connect to the Huddle audio relay.',
        cause: error,
      );
      _fail(failure, generation);
      throw failure;
    }
  }

  @override
  void sendOpusFrame({
    required HuddleAudioHeader header,
    required Uint8List opusPayload,
  }) {
    final channel = _channel;
    if (_disposed ||
        channel == null ||
        _state.phase != HuddleTransportPhase.connected) {
      throw HuddleTransportError(
        code: HuddleTransportErrorCode.invalidState,
        message: 'Cannot send audio while transport is ${_state.phase.name}.',
      );
    }

    try {
      channel.sink.add(HuddleWireV2.encodeClientFrame(header, opusPayload));
    } catch (error) {
      throw HuddleTransportError(
        code: HuddleTransportErrorCode.sendFailed,
        message: 'Unable to send Huddle audio.',
        cause: error,
      );
    }
  }

  @override
  Future<void> disconnect() async {
    if (_disposed ||
        _state.phase == HuddleTransportPhase.idle ||
        _state.phase == HuddleTransportPhase.disconnected) {
      return;
    }

    _generation++;
    _intentionalClose = true;
    _handshakeTimer?.cancel();
    _handshakeTimer = null;
    if (_handshakeCompleter case final completer? when !completer.isCompleted) {
      completer.completeError(
        const HuddleTransportError(
          code: HuddleTransportErrorCode.cancelled,
          message: 'Huddle connection was closed.',
        ),
      );
    }
    _emitState(HuddleTransportState(phase: HuddleTransportPhase.closing));

    final subscription = _subscription;
    _subscription = null;
    await subscription?.cancel();
    final channel = _channel;
    _channel = null;
    await channel?.sink.close();
    _handshakeCompleter = null;
    _emitState(HuddleTransportState(phase: HuddleTransportPhase.disconnected));
  }

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    await disconnect();
    _disposed = true;
    _audioIngressTimer?.cancel();
    _audioIngressTimer = null;
    _audioIngress.clear();
    await _stateController.close();
    await _audioController.close();
    await _peerController.close();
    await _issueController.close();
  }

  void _handleRawMessage(dynamic raw, int generation) {
    if (!_isCurrent(generation)) return;
    if (raw is String) {
      _handleTextMessage(raw, generation);
      return;
    }
    if (raw is List<int>) {
      _handleBinaryMessage(Uint8List.fromList(raw), generation);
      return;
    }
    _handleProtocolProblem('Unsupported Huddle WebSocket message.', generation);
  }

  void _handleTextMessage(String raw, int generation) {
    final Map<String, dynamic> message;
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map) throw const FormatException('expected object');
      message = Map<String, dynamic>.from(decoded);
    } catch (error) {
      _handleProtocolProblem('Malformed Huddle control message.', generation);
      return;
    }

    switch (message['type']) {
      case 'challenge':
        _handleChallenge(message, generation);
      case 'joined':
        _handleJoined(message, generation);
      case 'left':
        _handleLeft(message, generation);
      case 'roster':
        _handleRoster(message, generation);
      case 'error':
        _handleRelayError(message, generation);
      default:
        _handleProtocolProblem('Unknown Huddle control message.', generation);
    }
  }

  void _handleChallenge(Map<String, dynamic> message, int generation) {
    if (_state.phase != HuddleTransportPhase.awaitingChallenge) {
      _handleProtocolProblem('Unexpected Huddle auth challenge.', generation);
      return;
    }
    final challenge = message['challenge'];
    if (challenge is! String || challenge.isEmpty) {
      _fail(
        const HuddleTransportError(
          code: HuddleTransportErrorCode.protocolViolation,
          message: 'Huddle auth challenge was missing.',
        ),
        generation,
      );
      return;
    }

    try {
      final auth = HuddleAuthV2.buildMessage(
        parameters: parameters,
        challenge: challenge,
      );
      _channel?.sink.add(jsonEncode(auth));
      _emitState(
        HuddleTransportState(
          phase: HuddleTransportPhase.authenticating,
          localPeerIndex: _state.localPeerIndex,
          rosterRevision: _state.rosterRevision,
          peers: _state.peers,
        ),
      );
    } catch (error) {
      _fail(
        HuddleTransportError(
          code: HuddleTransportErrorCode.authenticationFailed,
          message: 'Unable to sign Huddle authentication.',
          cause: error,
        ),
        generation,
      );
    }
  }

  void _handleJoined(Map<String, dynamic> message, int generation) {
    if (_state.phase != HuddleTransportPhase.authenticating &&
        _state.phase != HuddleTransportPhase.connected) {
      _handleProtocolProblem('Unexpected Huddle joined message.', generation);
      return;
    }

    final pubkey = message['pubkey'];
    final peerIndex = message['peer_index'];
    final epoch = _parseEpoch(message['epoch']);
    if (pubkey is! String ||
        pubkey.isEmpty ||
        peerIndex is! int ||
        peerIndex < 0 ||
        peerIndex > 255 ||
        epoch == null) {
      _handleProtocolProblem('Malformed Huddle joined message.', generation);
      return;
    }

    final initialAdmission =
        _state.phase == HuddleTransportPhase.authenticating;
    final previousAdmissionPeers = initialAdmission
        ? (_admissionPreviousPeers ?? _state.peers)
        : _state.peers;
    final revision = message['revision'];
    if (revision != null && (revision is! int || revision < 0)) {
      _handleProtocolProblem('Malformed Huddle joined revision.', generation);
      return;
    }
    final currentRevision = _state.rosterRevision;
    final staleAdmission =
        initialAdmission &&
        currentRevision != null &&
        revision is int &&
        revision < currentRevision;
    final peers = Map<int, HuddlePeer>.from(_state.peers);
    final replaced = peers[peerIndex];
    final snapshot = message['peers'];
    if (snapshot is List) {
      final parsed = _parsePeerSnapshot(snapshot);
      if (parsed == null) {
        _handleProtocolProblem('Malformed Huddle joined roster.', generation);
        return;
      }
      if (initialAdmission && !staleAdmission) {
        peers
          ..clear()
          ..addAll(parsed);
      } else if (!initialAdmission && revision == null) {
        // Legacy, unrevisioned relays used this field as a best-effort roster
        // snapshot. Revisioned joined controls are deltas; merging their
        // payload before inspecting the occupied slot would hide index reuse.
        peers.addAll(parsed);
      }
    }
    if (!initialAdmission &&
        !_acceptDeltaRevision(revision as int?, generation)) {
      return;
    }
    if (initialAdmission && revision == null) {
      final selfPresent = peers.values.any(
        (candidate) => candidate.pubkey.toLowerCase() == pubkey.toLowerCase(),
      );
      if (!selfPresent) {
        peers[peerIndex] = HuddlePeer(
          pubkey: pubkey,
          peerIndex: peerIndex,
          epoch: epoch,
        );
      }
    }
    final peer = HuddlePeer(pubkey: pubkey, peerIndex: peerIndex, epoch: epoch);
    if (!initialAdmission || revision == null) {
      peers[peerIndex] = peer;
    }
    if (initialAdmission && _hasAdmitted) {
      _emitAdmissionRosterChanges(previousAdmissionPeers, peers);
    }
    if (!initialAdmission &&
        replaced != null &&
        !_sameOccupancy(replaced, peer)) {
      _purgeAudioIngress({peerIndex});
    }

    _emitState(
      HuddleTransportState(
        phase: HuddleTransportPhase.connected,
        localPeerIndex: initialAdmission ? peerIndex : _state.localPeerIndex,
        rosterRevision: initialAdmission
            ? (staleAdmission ? currentRevision : revision as int?)
            : (revision as int? ?? _state.rosterRevision),
        peers: peers,
      ),
    );
    if (!initialAdmission) {
      if (replaced != null && !_sameOccupancy(replaced, peer)) {
        _peerController.add(
          HuddlePeerEvent(
            type: HuddlePeerEventType.replaced,
            peer: replaced,
            replacement: peer,
          ),
        );
      } else {
        _peerController.add(
          HuddlePeerEvent(type: HuddlePeerEventType.joined, peer: peer),
        );
      }
    }
    _handshakeTimer?.cancel();
    _handshakeTimer = null;
    if (initialAdmission) {
      _hasAdmitted = true;
      _admissionPreviousPeers = null;
    }
    if (_handshakeCompleter case final completer? when !completer.isCompleted) {
      completer.complete();
    }
  }

  void _emitAdmissionRosterChanges(
    Map<int, HuddlePeer> previous,
    Map<int, HuddlePeer> current,
  ) {
    final removedOrReplacedIndices = <int>{};
    for (final entry in previous.entries) {
      final replacement = current[entry.key];
      // A fresh socket admission may follow a relay restart that reused the
      // same pubkey/index/epoch while resetting its Opus sequence. Native
      // playout survives reconnects, so every re-admitted remote slot must
      // reset that prior decoder and jitter state.
      if (replacement == null) {
        removedOrReplacedIndices.add(entry.key);
        _peerController.add(
          HuddlePeerEvent(type: HuddlePeerEventType.left, peer: entry.value),
        );
      } else {
        removedOrReplacedIndices.add(entry.key);
        _peerController.add(
          HuddlePeerEvent(
            type: HuddlePeerEventType.replaced,
            peer: entry.value,
            replacement: replacement,
          ),
        );
      }
    }
    _purgeAudioIngress(removedOrReplacedIndices);
  }

  void _handleLeft(Map<String, dynamic> message, int generation) {
    if (_state.phase != HuddleTransportPhase.connected) {
      _handleProtocolProblem('Unexpected Huddle left message.', generation);
      return;
    }
    final pubkey = message['pubkey'];
    final peerIndex = message['peer_index'];
    final revision = message['revision'];
    if (pubkey is! String ||
        peerIndex is! int ||
        peerIndex < 0 ||
        peerIndex > 255 ||
        (revision != null && (revision is! int || revision < 0))) {
      _handleProtocolProblem('Malformed Huddle left message.', generation);
      return;
    }
    if (!_acceptDeltaRevision(revision as int?, generation)) return;

    final existing = _state.peers[peerIndex];
    if (existing != null && existing.pubkey != pubkey) {
      _requestRosterResync(generation);
      return;
    }
    final peers = Map<int, HuddlePeer>.from(_state.peers);
    final removed = peers.remove(peerIndex);
    if (removed == null) return;
    _purgeAudioIngress({peerIndex});
    _emitState(
      HuddleTransportState(
        phase: HuddleTransportPhase.connected,
        localPeerIndex: _state.localPeerIndex,
        rosterRevision: revision ?? _state.rosterRevision,
        peers: peers,
      ),
    );
    _peerController.add(
      HuddlePeerEvent(type: HuddlePeerEventType.left, peer: removed),
    );
  }

  void _handleRoster(Map<String, dynamic> message, int generation) {
    if (_state.phase == HuddleTransportPhase.authenticating) {
      _handleInitialRoster(message, generation);
      return;
    }
    if (_state.phase != HuddleTransportPhase.connected) {
      _handleProtocolProblem('Unexpected Huddle roster message.', generation);
      return;
    }
    final revision = message['revision'];
    final snapshot = message['peers'];
    if (revision is! int || revision < 0 || snapshot is! List) {
      _handleProtocolProblem('Malformed Huddle roster message.', generation);
      return;
    }
    final previousRevision = _state.rosterRevision;
    if (previousRevision != null && revision <= previousRevision) return;
    final peers = _parsePeerSnapshot(snapshot);
    if (peers == null) {
      _handleProtocolProblem('Malformed Huddle roster message.', generation);
      return;
    }

    final previous = _state.peers;
    final removedOrReplacedIndices = <int>{};
    for (final entry in previous.entries) {
      final replacement = peers[entry.key];
      if (replacement == null || !_sameOccupancy(replacement, entry.value)) {
        removedOrReplacedIndices.add(entry.key);
      }
    }
    _purgeAudioIngress(removedOrReplacedIndices);
    for (final entry in previous.entries) {
      final replacement = peers[entry.key];
      if (replacement == null) {
        _peerController.add(
          HuddlePeerEvent(type: HuddlePeerEventType.left, peer: entry.value),
        );
      } else if (!_sameOccupancy(replacement, entry.value)) {
        _peerController.add(
          HuddlePeerEvent(
            type: HuddlePeerEventType.replaced,
            peer: entry.value,
            replacement: replacement,
          ),
        );
      }
    }
    for (final entry in peers.entries) {
      if (!previous.containsKey(entry.key)) {
        _peerController.add(
          HuddlePeerEvent(type: HuddlePeerEventType.joined, peer: entry.value),
        );
      }
    }
    _emitState(
      HuddleTransportState(
        phase: HuddleTransportPhase.connected,
        localPeerIndex: _state.localPeerIndex,
        rosterRevision: revision,
        peers: peers,
      ),
    );
  }

  void _handleInitialRoster(Map<String, dynamic> message, int generation) {
    final revision = message['revision'];
    final snapshot = message['peers'];
    if (revision is! int || revision < 0 || snapshot is! List) {
      _handleProtocolProblem('Malformed Huddle roster message.', generation);
      return;
    }
    final peers = _parsePeerSnapshot(snapshot);
    if (peers == null) {
      _handleProtocolProblem('Malformed Huddle roster message.', generation);
      return;
    }
    // Mesh owners may repair a snapshot-to-delta race before the admission
    // message reaches this client. Retain that newer authoritative snapshot;
    // the following joined admission is allowed to establish localPeerIndex
    // without replacing it with older state.
    _emitState(
      HuddleTransportState(
        phase: HuddleTransportPhase.authenticating,
        rosterRevision: revision,
        peers: peers,
      ),
    );
  }

  bool _acceptDeltaRevision(int? revision, int generation) {
    final current = _state.rosterRevision;
    // Single-node relays do not revision their legacy joined/left controls.
    // Once a revisioned snapshot/delta is observed, every later delta must be
    // exactly sequential; gaps are repaired by an authoritative snapshot.
    if (revision == null) return current == null;
    if (current == null || revision == current + 1) return true;
    if (revision <= current) return false;
    _requestRosterResync(generation);
    return false;
  }

  void _requestRosterResync(int generation) {
    if (!_isCurrent(generation) ||
        _state.phase != HuddleTransportPhase.connected) {
      return;
    }
    _fail(
      const HuddleTransportError(
        code: HuddleTransportErrorCode.protocolViolation,
        message: 'Huddle roster revision gap; reconnecting for a fresh roster.',
      ),
      generation,
    );
  }

  Map<int, HuddlePeer>? _parsePeerSnapshot(List<dynamic> snapshot) {
    final peers = <int, HuddlePeer>{};
    for (final candidate in snapshot) {
      if (candidate is! Map) return null;
      final pubkey = candidate['pubkey'];
      final peerIndex = candidate['peer_index'];
      final epoch = _parseEpoch(candidate['epoch']);
      if (pubkey is! String ||
          pubkey.isEmpty ||
          peerIndex is! int ||
          peerIndex < 0 ||
          peerIndex > 255 ||
          epoch == null ||
          peers.containsKey(peerIndex)) {
        return null;
      }
      peers[peerIndex] = HuddlePeer(
        pubkey: pubkey,
        peerIndex: peerIndex,
        epoch: epoch,
      );
    }
    return peers;
  }

  static bool _sameOccupancy(HuddlePeer left, HuddlePeer right) =>
      left.pubkey == right.pubkey && left.epoch == right.epoch;

  /// Parse an optional wire `epoch` field. A current relay always sends it; a
  /// missing value defaults to `0` so the fence degrades to a no-op against a
  /// legacy relay rather than rejecting every frame. A present-but-malformed
  /// value (wrong type or out of the `u8` range) is a protocol error and
  /// yields `null`, mirroring `peer_index` strictness.
  static int? _parseEpoch(Object? raw) {
    if (raw == null) return 0;
    if (raw is! int || raw < 0 || raw > 255) return null;
    return raw;
  }

  void _handleRelayError(Map<String, dynamic> message, int generation) {
    final relayCode = message['code'] is String
        ? message['code'] as String
        : null;
    final relayMessage = message['message'] is String
        ? message['message'] as String
        : 'Huddle audio relay rejected the connection.';
    _fail(
      HuddleTransportError(
        code: HuddleTransportErrorCode.relayRejected,
        message: relayMessage,
        relayCode: relayCode,
      ),
      generation,
    );
  }

  void _handleBinaryMessage(Uint8List bytes, int generation) {
    if (_state.phase != HuddleTransportPhase.connected) {
      _handleProtocolProblem(
        'Received Huddle audio before room admission.',
        generation,
      );
      return;
    }
    try {
      final frame = HuddleWireV2.decodeRelayFrame(bytes);
      // The authoritative control roster owns the routing table. Protocol v2
      // carries only the peer index, so packets are accepted only while that
      // index is currently present. V3's stricter occupancy-epoch fence is not
      // available in this compatibility build.
      if (!_isCurrentOccupant(frame.peerIndex, frame.epoch)) return;
      if (_audioIngress.length == _audioIngressCapacity) {
        _audioIngress.removeFirst();
      }
      _audioIngress.addLast(frame);
      _audioIngressTimer ??= Timer(Duration.zero, _drainAudioIngress);
    } on HuddleWireException catch (error) {
      _issueController.add(
        HuddleTransportError(
          code: HuddleTransportErrorCode.protocolViolation,
          message: error.message,
          cause: error,
        ),
      );
    }
  }

  /// Whether `peerIndex` is currently occupied. Protocol v2 has no media epoch,
  /// so the control-plane roster is the strongest routing boundary available.
  bool _isCurrentOccupant(int peerIndex, int _) =>
      _state.peers.containsKey(peerIndex);

  void _purgeAudioIngress(Set<int> peerIndices) {
    if (peerIndices.isEmpty || _audioIngress.isEmpty) return;
    _audioIngress.removeWhere((frame) => peerIndices.contains(frame.peerIndex));
  }

  void _drainAudioIngress() {
    _audioIngressTimer = null;
    if (_disposed) {
      _audioIngress.clear();
      return;
    }
    while (_audioIngress.isNotEmpty) {
      final frame = _audioIngress.removeFirst();
      // Control and media are handled by the same socket callback, but queued
      // media drains on later event-loop turns. Revalidate here so a removal
      // or index replacement wins over every frame queued before that control.
      // Revalidate occupancy after the event-loop hop so a control-plane leave
      // can purge queued v2 media before it reaches native playout.
      if (_isCurrentOccupant(frame.peerIndex, frame.epoch)) {
        _audioController.add(frame);
        break;
      }
    }
    if (_audioIngress.isNotEmpty) {
      _audioIngressTimer = Timer(Duration.zero, _drainAudioIngress);
    }
  }

  void _handleProtocolProblem(String message, int generation) {
    final error = HuddleTransportError(
      code: HuddleTransportErrorCode.protocolViolation,
      message: message,
    );
    if (_state.phase == HuddleTransportPhase.connected) {
      _issueController.add(error);
    } else {
      _fail(error, generation);
    }
  }

  void _handleSocketFailure(Object error, int generation) {
    if (_intentionalClose || !_isCurrent(generation)) return;
    _fail(
      HuddleTransportError(
        code: HuddleTransportErrorCode.connectionFailed,
        message: 'Huddle audio socket failed.',
        cause: error,
      ),
      generation,
    );
  }

  void _handleSocketClosed(int generation) {
    if (_intentionalClose || !_isCurrent(generation)) return;
    _fail(
      const HuddleTransportError(
        code: HuddleTransportErrorCode.socketClosed,
        message: 'Huddle audio socket closed unexpectedly.',
      ),
      generation,
    );
  }

  void _fail(HuddleTransportError error, int generation) {
    if (!_isCurrent(generation) ||
        _state.phase == HuddleTransportPhase.failed) {
      return;
    }
    _handshakeTimer?.cancel();
    _handshakeTimer = null;
    _emitState(
      HuddleTransportState(
        phase: HuddleTransportPhase.failed,
        localPeerIndex: _state.localPeerIndex,
        rosterRevision: _state.rosterRevision,
        peers: _state.peers,
        error: error,
      ),
    );
    if (_handshakeCompleter case final completer? when !completer.isCompleted) {
      completer.completeError(error);
    }
    _subscription?.cancel();
    _subscription = null;
    final channel = _channel;
    _channel = null;
    unawaited(channel?.sink.close() ?? Future<void>.value());
  }

  bool _isCurrent(int generation) => !_disposed && generation == _generation;

  void _emitState(HuddleTransportState next) {
    _state = next;
    if (!_stateController.isClosed) _stateController.add(next);
  }
}
