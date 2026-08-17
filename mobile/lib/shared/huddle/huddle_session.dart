import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'huddle_auth.dart';
import 'huddle_media.dart';
import 'huddle_transport.dart';
import 'huddle_wire.dart';

enum HuddleSessionPhase {
  idle,
  checkingSupport,
  requestingPermission,
  preparingMedia,
  connecting,
  reconnecting,
  interrupted,
  connected,
  leaving,
  failed,
}

const _notProvided = Object();

@immutable
final class HuddleSessionState {
  final HuddleSessionPhase phase;
  final String? parentChannelId;
  final String? ephemeralChannelId;
  final String? startedEventId;
  final String? currentPubkey;
  final bool isCreator;

  /// Whether the audio relay admitted this identity during this session.
  final bool wasAdmitted;
  final bool isMuted;
  final bool isSpeakerEnabled;
  final int participantCount;
  final List<String> participantPubkeys;
  final Set<String> activeSpeakerPubkeys;
  final Map<String, double> speakerLevels;
  final int reconnectAttempt;
  final int receivedFrameCount;
  final int sentFrameCount;
  final String? issue;
  final String? error;

  const HuddleSessionState({
    required this.phase,
    this.parentChannelId,
    this.ephemeralChannelId,
    this.startedEventId,
    this.currentPubkey,
    this.isCreator = false,
    this.wasAdmitted = false,
    this.isMuted = false,
    this.isSpeakerEnabled = false,
    this.participantCount = 0,
    this.participantPubkeys = const [],
    this.activeSpeakerPubkeys = const {},
    this.speakerLevels = const {},
    this.reconnectAttempt = 0,
    this.receivedFrameCount = 0,
    this.sentFrameCount = 0,
    this.issue,
    this.error,
  });

  static const idle = HuddleSessionState(phase: HuddleSessionPhase.idle);

  bool get isConnected => phase == HuddleSessionPhase.connected;

  bool get isInSession => switch (phase) {
    HuddleSessionPhase.checkingSupport ||
    HuddleSessionPhase.requestingPermission ||
    HuddleSessionPhase.preparingMedia ||
    HuddleSessionPhase.connecting ||
    HuddleSessionPhase.reconnecting ||
    HuddleSessionPhase.interrupted ||
    HuddleSessionPhase.connected ||
    HuddleSessionPhase.leaving => true,
    HuddleSessionPhase.idle || HuddleSessionPhase.failed => false,
  };

  HuddleSessionState copyWith({
    HuddleSessionPhase? phase,
    Object? parentChannelId = _notProvided,
    Object? ephemeralChannelId = _notProvided,
    Object? startedEventId = _notProvided,
    Object? currentPubkey = _notProvided,
    bool? isCreator,
    bool? wasAdmitted,
    bool? isMuted,
    bool? isSpeakerEnabled,
    int? participantCount,
    List<String>? participantPubkeys,
    Set<String>? activeSpeakerPubkeys,
    Map<String, double>? speakerLevels,
    int? reconnectAttempt,
    int? receivedFrameCount,
    int? sentFrameCount,
    Object? issue = _notProvided,
    Object? error = _notProvided,
  }) => HuddleSessionState(
    phase: phase ?? this.phase,
    parentChannelId: parentChannelId == _notProvided
        ? this.parentChannelId
        : parentChannelId as String?,
    ephemeralChannelId: ephemeralChannelId == _notProvided
        ? this.ephemeralChannelId
        : ephemeralChannelId as String?,
    startedEventId: startedEventId == _notProvided
        ? this.startedEventId
        : startedEventId as String?,
    currentPubkey: currentPubkey == _notProvided
        ? this.currentPubkey
        : currentPubkey as String?,
    isCreator: isCreator ?? this.isCreator,
    wasAdmitted: wasAdmitted ?? this.wasAdmitted,
    isMuted: isMuted ?? this.isMuted,
    isSpeakerEnabled: isSpeakerEnabled ?? this.isSpeakerEnabled,
    participantCount: participantCount ?? this.participantCount,
    participantPubkeys: participantPubkeys ?? this.participantPubkeys,
    activeSpeakerPubkeys: activeSpeakerPubkeys ?? this.activeSpeakerPubkeys,
    speakerLevels: speakerLevels ?? this.speakerLevels,
    reconnectAttempt: reconnectAttempt ?? this.reconnectAttempt,
    receivedFrameCount: receivedFrameCount ?? this.receivedFrameCount,
    sentFrameCount: sentFrameCount ?? this.sentFrameCount,
    issue: issue == _notProvided ? this.issue : issue as String?,
    error: error == _notProvided ? this.error : error as String?,
  );
}

typedef HuddleMediaFactory = HuddleMedia Function();
typedef HuddleTransportFactory =
    HuddleTransportClient Function(HuddleConnectionParameters parameters);

final huddleMediaFactoryProvider = Provider<HuddleMediaFactory>(
  (_) => MethodChannelHuddleMedia.new,
);

final huddleTransportFactoryProvider = Provider<HuddleTransportFactory>(
  (_) =>
      (parameters) => HuddleTransport(parameters: parameters),
);

final huddleReconnectDelaysProvider = Provider<List<Duration>>(
  (_) => const [
    Duration.zero,
    Duration(milliseconds: 100),
    Duration(milliseconds: 250),
    Duration(milliseconds: 500),
    Duration(seconds: 1),
    Duration(seconds: 2),
    Duration(seconds: 2),
  ],
);

final huddleSessionProvider =
    NotifierProvider<HuddleSessionNotifier, HuddleSessionState>(
      HuddleSessionNotifier.new,
    );

/// Owns one foreground mobile Huddle and wires native Opus media to its socket.
final class HuddleSessionNotifier extends Notifier<HuddleSessionState> {
  HuddleMedia? _media;
  HuddleTransportClient? _transport;
  StreamSubscription<HuddleMediaState>? _mediaStateSubscription;
  StreamSubscription<HuddleLocalAudioFrame>? _localFrameSubscription;
  StreamSubscription<HuddleTransportState>? _transportStateSubscription;
  StreamSubscription<HuddleRemoteAudioFrame>? _remoteFrameSubscription;
  StreamSubscription<HuddleTransportError>? _transportIssueSubscription;
  StreamSubscription<HuddlePeerEvent>? _peerEventSubscription;
  var _generation = 0;
  var _receivedFrames = 0;
  var _sentFrames = 0;
  Future<void> _playbackTail = Future<void>.value();
  final Map<String, Timer> _speakerTimers = {};
  final Map<String, double> _pendingSpeakerLevels = {};
  Timer? _speakerLevelFlushTimer;
  Timer? _reconnectTimer;
  var _reconnectAttempt = 0;
  var _reconnectInFlight = false;
  var _wasConnected = false;

  @override
  HuddleSessionState build() {
    ref.onDispose(() {
      _generation += 1;
      unawaited(_disposeResources());
    });
    return HuddleSessionState.idle;
  }

  Future<void> join(
    HuddleConnectionParameters parameters, {
    String? currentPubkey,
    bool isCreator = false,
    String? startedEventId,
  }) async {
    if (state.isInSession) {
      if (state.ephemeralChannelId == parameters.ephemeralChannelId) return;
      state = state.copyWith(
        error: 'Leave the current Huddle before joining another one.',
      );
      return;
    }

    final generation = ++_generation;
    await _disposeResources();
    if (!_isCurrent(generation)) return;
    _receivedFrames = 0;
    _sentFrames = 0;
    _reconnectAttempt = 0;
    _reconnectInFlight = false;
    _wasConnected = false;
    state = HuddleSessionState(
      phase: HuddleSessionPhase.checkingSupport,
      parentChannelId: parameters.parentChannelId,
      ephemeralChannelId: parameters.ephemeralChannelId,
      startedEventId: startedEventId,
      currentPubkey: currentPubkey?.toLowerCase(),
      isCreator: isCreator,
    );

    try {
      final media = ref.read(huddleMediaFactoryProvider)();
      _media = media;
      _mediaStateSubscription = media.states.listen((mediaState) {
        if (!_isCurrent(generation)) return;
        if (mediaState.phase == HuddleMediaPhase.failed) {
          unawaited(
            _fail(
              mediaState.error?.message ?? 'Native Huddle audio failed.',
              generation,
            ),
          );
          return;
        }
        if (mediaState.phase != HuddleMediaPhase.active) return;
        if (mediaState.isInterrupted && state.isConnected) {
          state = state.copyWith(phase: HuddleSessionPhase.interrupted);
        } else if (!mediaState.isInterrupted &&
            state.phase == HuddleSessionPhase.interrupted) {
          state = state.copyWith(phase: HuddleSessionPhase.connected);
        }
      });

      final capabilities = await media.discoverCapabilities();
      _ensureCurrent(generation);
      if (!capabilities.supportsFullDuplexOpus) {
        throw const HuddleMediaError(
          code: HuddleMediaErrorCode.unsupported,
          message: 'Full-duplex mobile Huddle audio is unavailable here.',
        );
      }

      state = state.copyWith(phase: HuddleSessionPhase.requestingPermission);
      final permission = await media.requestMicrophonePermission();
      _ensureCurrent(generation);
      if (permission != HuddleMicrophonePermission.granted) {
        throw const HuddleMediaError(
          code: HuddleMediaErrorCode.permissionDenied,
          message: 'Microphone permission is required to join a Huddle.',
        );
      }

      state = state.copyWith(phase: HuddleSessionPhase.preparingMedia);
      await media.prepare();
      _ensureCurrent(generation);
      await media.start();
      _ensureCurrent(generation);

      final transport = ref.read(huddleTransportFactoryProvider)(parameters);
      _transport = transport;
      _wireMediaAndTransport(media, transport, generation);
      state = state.copyWith(
        phase: HuddleSessionPhase.connecting,
        isMuted: false,
      );
      await transport.connect();
      _ensureCurrent(generation);
      state = state.copyWith(
        phase: media.state.isInterrupted
            ? HuddleSessionPhase.interrupted
            : HuddleSessionPhase.connected,
        isMuted: media.state.isMuted,
        isSpeakerEnabled: media.state.isSpeakerEnabled,
        wasAdmitted: true,
        participantCount: transport.state.peers.length,
        participantPubkeys: _participantPubkeys(transport.state),
        reconnectAttempt: 0,
        issue: null,
        error: null,
      );
    } catch (error) {
      if (_isCurrent(generation)) {
        await _fail(_messageFor(error), generation);
      }
    }
  }

  Future<void> setMuted(bool muted) async {
    if (state.phase != HuddleSessionPhase.connected &&
        state.phase != HuddleSessionPhase.interrupted) {
      return;
    }
    final generation = _generation;
    final media = _media;
    if (media == null) return;
    try {
      await media.setMuted(muted);
      if (_isCurrent(generation)) state = state.copyWith(isMuted: muted);
    } catch (error) {
      if (_isCurrent(generation)) {
        await _fail(_messageFor(error), generation);
      }
    }
  }

  Future<void> setSpeakerEnabled(bool enabled) async {
    if (state.phase != HuddleSessionPhase.connected &&
        state.phase != HuddleSessionPhase.interrupted) {
      return;
    }
    final generation = _generation;
    final media = _media;
    if (media == null) return;
    try {
      await media.setSpeakerEnabled(enabled);
      if (_isCurrent(generation)) {
        state = state.copyWith(isSpeakerEnabled: enabled);
      }
    } catch (error) {
      if (_isCurrent(generation)) {
        await _fail(_messageFor(error), generation);
      }
    }
  }

  Future<void> leave() async {
    if (state.phase == HuddleSessionPhase.idle ||
        state.phase == HuddleSessionPhase.leaving) {
      return;
    }
    _generation += 1;
    state = state.copyWith(phase: HuddleSessionPhase.leaving, error: null);
    try {
      await _disposeResources();
    } finally {
      state = HuddleSessionState.idle;
    }
  }

  void _wireMediaAndTransport(
    HuddleMedia media,
    HuddleTransportClient transport,
    int generation,
  ) {
    _localFrameSubscription = media.localAudioFrames.listen((frame) {
      if (!_isCurrent(generation) ||
          state.phase != HuddleSessionPhase.connected ||
          state.isMuted) {
        return;
      }
      try {
        transport.sendOpusFrame(
          header: frame.header,
          opusPayload: frame.opusPayload,
        );
        if (!frame.header.isDtx && frame.header.levelDbov >= -55) {
          final currentPubkey = state.currentPubkey;
          if (currentPubkey != null) {
            _recordSpeakerPubkey(
              currentPubkey,
              frame.header.levelDbov,
              generation,
            );
          }
        }
        _sentFrames += 1;
        _emitStatsIfNeeded(sent: true);
      } catch (error) {
        if (_wasConnected) {
          _scheduleReconnect(transport, generation, _messageFor(error));
        } else {
          unawaited(_fail(_messageFor(error), generation));
        }
      }
    });
    _remoteFrameSubscription = transport.remoteAudioFrames.listen((frame) {
      if (!_isCurrent(generation)) return;
      _recordSpeaker(frame, transport, generation);
      _receivedFrames += 1;
      _emitStatsIfNeeded(sent: false);
      _playbackTail = _playbackTail.then((_) => media.playRemoteFrame(frame));
      unawaited(
        _playbackTail.catchError((Object error) async {
          if (_isCurrent(generation)) {
            await _fail(_messageFor(error), generation);
          }
        }),
      );
    });
    _transportStateSubscription = transport.states.listen((transportState) {
      if (!_isCurrent(generation)) return;
      if (transportState.phase == HuddleTransportPhase.connected) {
        _wasConnected = true;
        _reconnectTimer?.cancel();
        _reconnectTimer = null;
        _reconnectAttempt = 0;
        _reconnectInFlight = false;
        state = state.copyWith(
          phase: media.state.isInterrupted
              ? HuddleSessionPhase.interrupted
              : HuddleSessionPhase.connected,
          wasAdmitted: true,
          participantCount: transportState.peers.length,
          participantPubkeys: _participantPubkeys(transportState),
          reconnectAttempt: 0,
          issue: null,
          error: null,
        );
      } else if (transportState.phase == HuddleTransportPhase.failed) {
        final message =
            transportState.error?.message ?? 'Huddle connection failed.';
        if (_wasConnected) {
          _scheduleReconnect(transport, generation, message);
        } else {
          unawaited(_fail(message, generation));
        }
      }
    });
    _peerEventSubscription = transport.peerEvents.listen((event) {
      if (!_isCurrent(generation)) return;
      if (event.type == HuddlePeerEventType.left) {
        _clearSpeaker(event.peer.pubkey);
      }
    });
    _transportIssueSubscription = transport.issues.listen((issue) {
      if (_isCurrent(generation)) state = state.copyWith(issue: issue.message);
    });
  }

  List<String> _participantPubkeys(HuddleTransportState transportState) {
    final peers = transportState.peers.values.toList()
      ..sort((left, right) => left.peerIndex.compareTo(right.peerIndex));
    return List.unmodifiable(peers.map((peer) => peer.pubkey.toLowerCase()));
  }

  void _recordSpeaker(
    HuddleRemoteAudioFrame frame,
    HuddleTransportClient transport,
    int generation,
  ) {
    if (frame.header.isDtx || frame.header.levelDbov < -55) return;
    final pubkey = transport.state.peers[frame.peerIndex]?.pubkey.toLowerCase();
    if (pubkey == null) return;
    _recordSpeakerPubkey(pubkey, frame.header.levelDbov, generation);
  }

  void _recordSpeakerPubkey(String pubkey, int levelDbov, int generation) {
    final normalized = pubkey.toLowerCase();
    final wasActive = state.activeSpeakerPubkeys.contains(normalized);
    final level = _speakerLevelFromDbov(levelDbov);
    if (!wasActive) {
      final speakers = {...state.activeSpeakerPubkeys, normalized};
      final levels = {...state.speakerLevels, normalized: level};
      state = state.copyWith(
        activeSpeakerPubkeys: Set.unmodifiable(speakers),
        speakerLevels: Map.unmodifiable(levels),
      );
    } else {
      _pendingSpeakerLevels[normalized] = level;
      _scheduleSpeakerLevelFlush(generation);
    }
    _speakerTimers.remove(normalized)?.cancel();
    _speakerTimers[normalized] = Timer(const Duration(milliseconds: 600), () {
      if (!_isCurrent(generation)) return;
      _clearSpeaker(normalized);
    });
  }

  void _clearSpeaker(String pubkey) {
    final normalized = pubkey.toLowerCase();
    _speakerTimers.remove(normalized)?.cancel();
    _pendingSpeakerLevels.remove(normalized);
    if (!state.activeSpeakerPubkeys.contains(normalized) &&
        !state.speakerLevels.containsKey(normalized)) {
      return;
    }
    final levels = Map<String, double>.from(state.speakerLevels)
      ..remove(normalized);
    state = state.copyWith(
      activeSpeakerPubkeys: Set.unmodifiable(
        state.activeSpeakerPubkeys.where((value) => value != normalized),
      ),
      speakerLevels: Map.unmodifiable(levels),
    );
  }

  void _scheduleSpeakerLevelFlush(int generation) {
    if (_speakerLevelFlushTimer != null) return;
    _speakerLevelFlushTimer = Timer(const Duration(milliseconds: 50), () {
      _speakerLevelFlushTimer = null;
      if (!_isCurrent(generation)) {
        _pendingSpeakerLevels.clear();
        return;
      }
      if (_pendingSpeakerLevels.isEmpty) return;
      final levels = {...state.speakerLevels, ..._pendingSpeakerLevels};
      _pendingSpeakerLevels.clear();
      state = state.copyWith(speakerLevels: Map.unmodifiable(levels));
    });
  }

  void _scheduleReconnect(
    HuddleTransportClient transport,
    int generation,
    String issue,
  ) {
    if (!_isCurrent(generation) ||
        _reconnectInFlight ||
        _reconnectTimer != null ||
        state.phase == HuddleSessionPhase.leaving) {
      return;
    }
    final delays = ref.read(huddleReconnectDelaysProvider);
    if (_reconnectAttempt >= delays.length) {
      unawaited(_fail('Huddle connection could not be restored.', generation));
      return;
    }
    final attempt = ++_reconnectAttempt;
    state = state.copyWith(
      phase: HuddleSessionPhase.reconnecting,
      reconnectAttempt: attempt,
      issue: issue,
    );
    _reconnectTimer = Timer(delays[attempt - 1], () async {
      _reconnectTimer = null;
      if (!_isCurrent(generation)) return;
      _reconnectInFlight = true;
      try {
        await transport.connect();
      } catch (error) {
        if (!_isCurrent(generation)) return;
        _reconnectInFlight = false;
        _scheduleReconnect(transport, generation, _messageFor(error));
      }
    });
  }

  void _emitStatsIfNeeded({required bool sent}) {
    final count = sent ? _sentFrames : _receivedFrames;
    if (count != 1 && count % 10 != 0) return;
    state = state.copyWith(
      sentFrameCount: _sentFrames,
      receivedFrameCount: _receivedFrames,
    );
  }

  Future<void> _fail(String message, int generation) async {
    if (!_isCurrent(generation)) return;
    _generation += 1;
    state = state.copyWith(
      phase: HuddleSessionPhase.failed,
      isMuted: true,
      error: message,
    );
    await _disposeResources();
  }

  Future<void> _disposeResources() async {
    final subscriptions = <StreamSubscription<dynamic>?>[
      _mediaStateSubscription,
      _localFrameSubscription,
      _transportStateSubscription,
      _remoteFrameSubscription,
      _transportIssueSubscription,
      _peerEventSubscription,
    ];
    _mediaStateSubscription = null;
    _localFrameSubscription = null;
    _transportStateSubscription = null;
    _remoteFrameSubscription = null;
    _transportIssueSubscription = null;
    _peerEventSubscription = null;
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    _reconnectInFlight = false;
    for (final timer in _speakerTimers.values) {
      timer.cancel();
    }
    _speakerTimers.clear();
    _speakerLevelFlushTimer?.cancel();
    _speakerLevelFlushTimer = null;
    _pendingSpeakerLevels.clear();
    final transport = _transport;
    final media = _media;
    _transport = null;
    _media = null;

    // Stop native capture first. Cancelling a subscription to a synchronous
    // media stream can wait for an in-flight callback, which must never keep
    // the microphone active after the user hangs up. The generation guard
    // above makes any final state/frame callbacks harmless during disposal.
    Object? failure;
    StackTrace? failureStackTrace;
    try {
      await media?.dispose();
    } catch (error, stackTrace) {
      failure = error;
      failureStackTrace = stackTrace;
    }
    try {
      await transport?.dispose();
    } catch (error, stackTrace) {
      failure ??= error;
      failureStackTrace ??= stackTrace;
    }
    for (final subscription
        in subscriptions.whereType<StreamSubscription<dynamic>>()) {
      unawaited(
        subscription.cancel().catchError((Object error, StackTrace stackTrace) {
          debugPrint('Unable to detach a Huddle stream listener: $error');
        }),
      );
    }
    _playbackTail = Future<void>.value();
    if (failure != null) {
      Error.throwWithStackTrace(failure, failureStackTrace!);
    }
  }

  bool _isCurrent(int generation) => generation == _generation;

  void _ensureCurrent(int generation) {
    if (!_isCurrent(generation)) {
      throw const HuddleTransportError(
        code: HuddleTransportErrorCode.cancelled,
        message: 'Huddle join was cancelled.',
      );
    }
  }

  String _messageFor(Object error) => switch (error) {
    HuddleMediaError() => error.message,
    HuddleTransportError() => error.message,
    _ => 'Unable to join the Huddle.',
  };
}

double _speakerLevelFromDbov(int levelDbov) =>
    ((levelDbov + 55) / 55).clamp(0.0, 1.0).toDouble();
