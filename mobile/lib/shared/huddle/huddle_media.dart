import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'huddle_wire.dart';

const huddleMediaMethodChannelName = 'buzz/huddle_media';

enum HuddleMediaPhase {
  idle,
  discovering,
  preparing,
  prepared,
  active,
  stopping,
  stopped,
  unavailable,
  failed,
}

enum HuddleMicrophonePermission {
  undetermined,
  granted,
  denied,
  restricted,
  unavailable,
}

enum HuddleMediaErrorCode {
  invalidState,
  unavailable,
  permissionDenied,
  unsupported,
  platformFailure,
  malformedNativeEvent,
  disposed,
}

@immutable
final class HuddleMediaError implements Exception {
  final HuddleMediaErrorCode code;
  final String message;
  final Object? cause;

  const HuddleMediaError({
    required this.code,
    required this.message,
    this.cause,
  });

  @override
  String toString() => 'HuddleMediaError.${code.name}: $message';
}

/// Native media capabilities are deliberately granular.
///
/// Flutter joins only when every media capability is present. Android and iOS
/// report full-duplex support only after probing their platform Opus codecs.
/// This prevents a transport-only client from appearing in a room as a silent
/// participant.
@immutable
final class HuddleMediaCapabilities {
  final String platform;
  final bool supportsAudioSession;
  final bool supportsMicrophonePermission;
  final bool supportsCapture;
  final bool supportsPlayback;
  final bool supportsOpusEncoding;
  final bool supportsOpusDecoding;

  const HuddleMediaCapabilities({
    required this.platform,
    required this.supportsAudioSession,
    required this.supportsMicrophonePermission,
    required this.supportsCapture,
    required this.supportsPlayback,
    required this.supportsOpusEncoding,
    required this.supportsOpusDecoding,
  });

  static const unavailable = HuddleMediaCapabilities(
    platform: 'unavailable',
    supportsAudioSession: false,
    supportsMicrophonePermission: false,
    supportsCapture: false,
    supportsPlayback: false,
    supportsOpusEncoding: false,
    supportsOpusDecoding: false,
  );

  bool get supportsFullDuplexOpus =>
      supportsCapture &&
      supportsPlayback &&
      supportsOpusEncoding &&
      supportsOpusDecoding;

  factory HuddleMediaCapabilities.fromMap(Map<dynamic, dynamic> value) =>
      HuddleMediaCapabilities(
        platform: value['platform'] as String? ?? 'unknown',
        supportsAudioSession: value['audioSession'] == true,
        supportsMicrophonePermission: value['microphonePermission'] == true,
        supportsCapture: value['capture'] == true,
        supportsPlayback: value['playback'] == true,
        supportsOpusEncoding: value['opusEncoding'] == true,
        supportsOpusDecoding: value['opusDecoding'] == true,
      );
}

/// Aggregate debug telemetry for the microphone signal immediately before Opus.
///
/// It contains levels and platform configuration only. PCM, Opus packets, and
/// speech content are never retained by this diagnostic path.
@immutable
final class HuddleCaptureDiagnostics {
  final int frameCount;
  final Map<int, int> rmsDbovHistogram;
  final Map<int, int> peakDbovHistogram;
  final int maxPeakDbov;
  final int audioSource;
  final String audioSourceLabel;
  final int? deviceId;
  final int? deviceType;
  final String? deviceLabel;
  final bool acousticEchoCancelerAvailable;
  final bool? acousticEchoCancelerEnabled;
  final bool noiseSuppressorAvailable;
  final bool? noiseSuppressorEnabled;
  final bool automaticGainControlAvailable;
  final bool? automaticGainControlEnabled;

  const HuddleCaptureDiagnostics({
    required this.frameCount,
    required this.rmsDbovHistogram,
    required this.peakDbovHistogram,
    required this.maxPeakDbov,
    required this.audioSource,
    required this.audioSourceLabel,
    required this.deviceId,
    required this.deviceType,
    required this.deviceLabel,
    required this.acousticEchoCancelerAvailable,
    required this.acousticEchoCancelerEnabled,
    required this.noiseSuppressorAvailable,
    required this.noiseSuppressorEnabled,
    required this.automaticGainControlAvailable,
    required this.automaticGainControlEnabled,
  });

  factory HuddleCaptureDiagnostics.fromMap(
    Map<dynamic, dynamic> value,
  ) => HuddleCaptureDiagnostics(
    frameCount: (value['frameCount'] as num).toInt(),
    rmsDbovHistogram: _parseDbovHistogram(value['rmsDbovHistogram']),
    peakDbovHistogram: _parseDbovHistogram(value['peakDbovHistogram']),
    maxPeakDbov: (value['maxPeakDbov'] as num).toInt(),
    audioSource: (value['audioSource'] as num).toInt(),
    audioSourceLabel: value['audioSourceLabel'] as String,
    deviceId: (value['deviceId'] as num?)?.toInt(),
    deviceType: (value['deviceType'] as num?)?.toInt(),
    deviceLabel: value['deviceLabel'] as String?,
    acousticEchoCancelerAvailable:
        value['acousticEchoCancelerAvailable'] as bool,
    acousticEchoCancelerEnabled: value['acousticEchoCancelerEnabled'] as bool?,
    noiseSuppressorAvailable: value['noiseSuppressorAvailable'] as bool,
    noiseSuppressorEnabled: value['noiseSuppressorEnabled'] as bool?,
    automaticGainControlAvailable:
        value['automaticGainControlAvailable'] as bool,
    automaticGainControlEnabled: value['automaticGainControlEnabled'] as bool?,
  );

  String get debugSummary =>
      'frames=$frameCount rmsDbov=$rmsDbovHistogram '
      'peakDbov=$peakDbovHistogram maxPeakDbov=$maxPeakDbov '
      'source=$audioSourceLabel($audioSource) '
      'device=${deviceLabel ?? 'unknown'}($deviceType/$deviceId) '
      'aec=$acousticEchoCancelerEnabled/$acousticEchoCancelerAvailable '
      'ns=$noiseSuppressorEnabled/$noiseSuppressorAvailable '
      'agc=$automaticGainControlEnabled/$automaticGainControlAvailable';
}

Map<int, int> _parseDbovHistogram(Object? value) {
  if (value is! Map) throw const FormatException('missing dBov histogram');
  final result = <int, int>{};
  for (final entry in value.entries) {
    final level = int.tryParse(entry.key.toString());
    final count = entry.value;
    if (level == null || count is! num) {
      throw const FormatException('malformed dBov histogram');
    }
    result[level] = count.toInt();
  }
  return Map.unmodifiable(result);
}

@immutable
final class HuddleMediaState {
  final HuddleMediaPhase phase;
  final HuddleMediaCapabilities? capabilities;
  final bool isMuted;
  final bool isSpeakerEnabled;
  final bool isInterrupted;
  final HuddleCaptureDiagnostics? captureDiagnostics;
  final HuddleMediaError? error;

  const HuddleMediaState({
    required this.phase,
    this.capabilities,
    this.isMuted = false,
    this.isSpeakerEnabled = false,
    this.isInterrupted = false,
    this.captureDiagnostics,
    this.error,
  });
}

@immutable
final class HuddleLocalAudioFrame {
  final HuddleAudioHeader header;
  final Uint8List opusPayload;

  const HuddleLocalAudioFrame({
    required this.header,
    required this.opusPayload,
  });
}

abstract interface class HuddleMedia {
  HuddleMediaState get state;
  Stream<HuddleMediaState> get states;
  Stream<HuddleLocalAudioFrame> get localAudioFrames;

  Future<HuddleMediaCapabilities> discoverCapabilities();
  Future<HuddleMicrophonePermission> requestMicrophonePermission();

  /// Open the OS app-settings screen so the user can grant a microphone
  /// permission that was previously denied. Returns `true` when the settings
  /// UI was launched. A no-op-safe fallback returns `false` on platforms that
  /// cannot open settings (e.g. the plugin is missing).
  Future<bool> openSystemSettings();

  Future<void> prepare();
  Future<void> start();
  Future<void> setMuted(bool muted);
  Future<void> setSpeakerEnabled(bool enabled);
  Future<void> playRemoteFrame(HuddleRemoteAudioFrame frame);
  Future<void> removeRemotePeer(int peerIndex);
  Future<void> stop();
  Future<void> dispose();
}

/// Flutter-facing bridge for the native realtime media engine.
///
/// The method names and payloads are intentionally codec-agnostic except for
/// the fixed Huddle v2 constants. Native capture pushes `localOpusFrame` calls
/// into [_handleNativeCall], and Flutter sends those compressed frames through
/// [HuddleTransport] without moving realtime PCM through the Flutter bridge.
final class MethodChannelHuddleMedia implements HuddleMedia {
  final MethodChannel _channel;
  final StreamController<HuddleMediaState> _stateController =
      StreamController.broadcast(sync: true);
  final StreamController<HuddleLocalAudioFrame> _localFrameController =
      StreamController.broadcast(sync: true);

  HuddleMediaState _state = const HuddleMediaState(
    phase: HuddleMediaPhase.idle,
  );
  HuddleMediaCapabilities? _capabilities;
  bool _disposed = false;

  MethodChannelHuddleMedia({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel(huddleMediaMethodChannelName) {
    _channel.setMethodCallHandler(_handleNativeCall);
  }

  @override
  HuddleMediaState get state => _state;

  @override
  Stream<HuddleMediaState> get states => _stateController.stream;

  @override
  Stream<HuddleLocalAudioFrame> get localAudioFrames =>
      _localFrameController.stream;

  @override
  Future<HuddleMediaCapabilities> discoverCapabilities() async {
    _ensureNotDisposed();
    _emit(const HuddleMediaState(phase: HuddleMediaPhase.discovering));
    try {
      final result = await _channel.invokeMapMethod<dynamic, dynamic>(
        'getCapabilities',
      );
      if (result == null) {
        throw const HuddleMediaError(
          code: HuddleMediaErrorCode.platformFailure,
          message: 'Native Huddle media capabilities were missing.',
        );
      }
      final capabilities = HuddleMediaCapabilities.fromMap(result);
      _capabilities = capabilities;
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.idle,
          capabilities: capabilities,
          isMuted: _state.isMuted,
          isSpeakerEnabled: _state.isSpeakerEnabled,
          captureDiagnostics: _state.captureDiagnostics,
        ),
      );
      return capabilities;
    } on MissingPluginException {
      _capabilities = HuddleMediaCapabilities.unavailable;
      _emit(
        const HuddleMediaState(
          phase: HuddleMediaPhase.unavailable,
          capabilities: HuddleMediaCapabilities.unavailable,
        ),
      );
      return HuddleMediaCapabilities.unavailable;
    } on HuddleMediaError catch (error) {
      _fail(error);
      rethrow;
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<HuddleMicrophonePermission> requestMicrophonePermission() async {
    _ensureNotDisposed();
    try {
      final value = await _channel.invokeMethod<String>(
        'requestMicrophonePermission',
      );
      return switch (value) {
        'granted' => HuddleMicrophonePermission.granted,
        'denied' => HuddleMicrophonePermission.denied,
        'restricted' => HuddleMicrophonePermission.restricted,
        'undetermined' => HuddleMicrophonePermission.undetermined,
        _ => HuddleMicrophonePermission.unavailable,
      };
    } on MissingPluginException {
      return HuddleMicrophonePermission.unavailable;
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<bool> openSystemSettings() async {
    _ensureNotDisposed();
    try {
      final opened = await _channel.invokeMethod<bool>('openSystemSettings');
      return opened ?? false;
    } on MissingPluginException {
      return false;
    } on PlatformException {
      // Opening settings is a best-effort recovery affordance; a platform
      // failure here must not tear down the (already-failed) session.
      return false;
    }
  }

  @override
  Future<void> prepare() async {
    _ensureNotDisposed();
    if (_state.phase != HuddleMediaPhase.idle &&
        _state.phase != HuddleMediaPhase.stopped) {
      throw HuddleMediaError(
        code: HuddleMediaErrorCode.invalidState,
        message: 'Cannot prepare media while it is ${_state.phase.name}.',
      );
    }
    final capabilities = _capabilities ?? await discoverCapabilities();
    if (!capabilities.supportsAudioSession) {
      throw const HuddleMediaError(
        code: HuddleMediaErrorCode.unavailable,
        message: 'Native Huddle audio sessions are unavailable.',
      );
    }

    _emit(
      HuddleMediaState(
        phase: HuddleMediaPhase.preparing,
        capabilities: capabilities,
        isMuted: _state.isMuted,
        isSpeakerEnabled: _state.isSpeakerEnabled,
        captureDiagnostics: _state.captureDiagnostics,
      ),
    );
    try {
      final result = await _channel
          .invokeMapMethod<dynamic, dynamic>('prepare', {
            'protocolVersion': HuddleWireV2.protocolVersion,
            'sampleRateHz': HuddleWireV2.sampleRateHz,
            'channels': HuddleWireV2.channels,
            'frameSamples': HuddleWireV2.frameSamples,
          });
      if (result?['audioSessionPrepared'] != true) {
        throw const HuddleMediaError(
          code: HuddleMediaErrorCode.platformFailure,
          message: 'Native Huddle audio session was not prepared.',
        );
      }
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.prepared,
          capabilities: capabilities,
          isMuted: false,
          isSpeakerEnabled: _state.isSpeakerEnabled,
          captureDiagnostics: _state.captureDiagnostics,
        ),
      );
    } on HuddleMediaError catch (error) {
      _fail(error);
      rethrow;
    } on PlatformException catch (error) {
      final failure = error.code == 'microphone_permission_denied'
          ? HuddleMediaError(
              code: HuddleMediaErrorCode.permissionDenied,
              message: error.message ?? 'Microphone permission is required.',
              cause: error,
            )
          : _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<void> start() async {
    _ensureNotDisposed();
    if (_state.phase != HuddleMediaPhase.prepared) {
      throw HuddleMediaError(
        code: HuddleMediaErrorCode.invalidState,
        message: 'Cannot start media while it is ${_state.phase.name}.',
      );
    }
    final capabilities = _capabilities;
    if (capabilities == null || !capabilities.supportsFullDuplexOpus) {
      throw const HuddleMediaError(
        code: HuddleMediaErrorCode.unsupported,
        message: 'Native full-duplex Opus media is not implemented yet.',
      );
    }

    try {
      await _channel.invokeMethod<void>('start');
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.active,
          capabilities: capabilities,
          isMuted: false,
          isSpeakerEnabled: _state.isSpeakerEnabled,
          captureDiagnostics: _state.captureDiagnostics,
        ),
      );
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<void> setMuted(bool muted) async {
    _ensureNotDisposed();
    if (_state.phase != HuddleMediaPhase.active) {
      throw HuddleMediaError(
        code: HuddleMediaErrorCode.invalidState,
        message: 'Cannot change mute while media is ${_state.phase.name}.',
      );
    }
    try {
      await _channel.invokeMethod<void>('setMuted', {'muted': muted});
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.active,
          capabilities: _capabilities,
          isMuted: muted,
          isSpeakerEnabled: _state.isSpeakerEnabled,
          isInterrupted: _state.isInterrupted,
          captureDiagnostics: _state.captureDiagnostics,
        ),
      );
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<void> setSpeakerEnabled(bool enabled) async {
    _ensureNotDisposed();
    if (_state.phase != HuddleMediaPhase.active) {
      throw HuddleMediaError(
        code: HuddleMediaErrorCode.invalidState,
        message:
            'Cannot change audio output while media is ${_state.phase.name}.',
      );
    }
    try {
      await _channel.invokeMethod<void>('setSpeakerEnabled', {
        'enabled': enabled,
      });
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.active,
          capabilities: _capabilities,
          isMuted: _state.isMuted,
          isSpeakerEnabled: enabled,
          isInterrupted: _state.isInterrupted,
          captureDiagnostics: _state.captureDiagnostics,
        ),
      );
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<void> playRemoteFrame(HuddleRemoteAudioFrame frame) async {
    _ensureNotDisposed();
    if (_state.phase != HuddleMediaPhase.active) {
      throw HuddleMediaError(
        code: HuddleMediaErrorCode.invalidState,
        message: 'Cannot play audio while media is ${_state.phase.name}.',
      );
    }
    if (_capabilities?.supportsOpusDecoding != true) {
      throw const HuddleMediaError(
        code: HuddleMediaErrorCode.unsupported,
        message: 'Native Opus playout is not implemented yet.',
      );
    }
    try {
      await _channel.invokeMethod<void>('playRemoteOpusFrame', {
        'peerIndex': frame.peerIndex,
        'sequence': frame.header.sequence,
        'timestamp48k': frame.header.timestamp48k,
        'levelDbov': frame.header.levelDbov,
        'flags': frame.header.flags,
        'opus': frame.opusPayload,
      });
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<void> removeRemotePeer(int peerIndex) async {
    _ensureNotDisposed();
    if (_state.phase != HuddleMediaPhase.active) return;
    try {
      await _channel.invokeMethod<void>('removeRemotePeer', {
        'peerIndex': peerIndex,
      });
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<void> stop() async {
    if (_disposed ||
        _state.phase == HuddleMediaPhase.idle ||
        _state.phase == HuddleMediaPhase.stopped ||
        _state.phase == HuddleMediaPhase.unavailable) {
      return;
    }
    _emit(
      HuddleMediaState(
        phase: HuddleMediaPhase.stopping,
        capabilities: _capabilities,
        isMuted: _state.isMuted,
        isSpeakerEnabled: _state.isSpeakerEnabled,
        captureDiagnostics: _state.captureDiagnostics,
      ),
    );
    try {
      await _channel.invokeMethod<void>('stop');
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.stopped,
          capabilities: _capabilities,
          isMuted: _state.isMuted,
          isSpeakerEnabled: _state.isSpeakerEnabled,
          captureDiagnostics: _state.captureDiagnostics,
        ),
      );
    } on MissingPluginException {
      _emit(
        const HuddleMediaState(
          phase: HuddleMediaPhase.unavailable,
          capabilities: HuddleMediaCapabilities.unavailable,
        ),
      );
    } on PlatformException catch (error) {
      final failure = _platformFailure(error);
      _fail(failure);
      throw failure;
    }
  }

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    await stop();
    _disposed = true;
    _channel.setMethodCallHandler(null);
    await _stateController.close();
    await _localFrameController.close();
  }

  Future<dynamic> _handleNativeCall(MethodCall call) async {
    if (_disposed) return null;
    switch (call.method) {
      case 'localOpusFrame':
        final arguments = call.arguments;
        if (arguments is! Map) {
          _reportMalformedNativeEvent();
          return null;
        }
        try {
          final opus = arguments['opus'];
          if (opus is! Uint8List || opus.isEmpty) {
            throw const FormatException('missing Opus payload');
          }
          _localFrameController.add(
            HuddleLocalAudioFrame(
              header: HuddleAudioHeader.checked(
                sequence: arguments['sequence'] as int,
                timestamp48k: arguments['timestamp48k'] as int,
                levelDbov: arguments['levelDbov'] as int,
                flags: arguments['flags'] as int,
              ),
              opusPayload: Uint8List.fromList(opus),
            ),
          );
        } catch (_) {
          _reportMalformedNativeEvent();
        }
      case 'nativeError':
        final arguments = call.arguments;
        final message = arguments is Map ? arguments['message'] : null;
        _fail(
          HuddleMediaError(
            code: HuddleMediaErrorCode.platformFailure,
            message: message is String && message.isNotEmpty
                ? message
                : 'Native Huddle audio failed.',
          ),
        );
      case 'interruptionChanged':
        final arguments = call.arguments;
        final interrupted = arguments is Map ? arguments['interrupted'] : null;
        if (interrupted is! bool) {
          _reportMalformedNativeEvent();
          return null;
        }
        _emit(
          HuddleMediaState(
            phase: HuddleMediaPhase.active,
            capabilities: _capabilities,
            isMuted: _state.isMuted,
            isSpeakerEnabled: _state.isSpeakerEnabled,
            isInterrupted: interrupted,
            captureDiagnostics: _state.captureDiagnostics,
          ),
        );
      case 'captureDiagnostics':
        final arguments = call.arguments;
        if (arguments is! Map) {
          _reportMalformedNativeEvent();
          return null;
        }
        try {
          final diagnostics = HuddleCaptureDiagnostics.fromMap(arguments);
          _emit(
            HuddleMediaState(
              phase: _state.phase,
              capabilities: _capabilities,
              isMuted: _state.isMuted,
              isSpeakerEnabled: _state.isSpeakerEnabled,
              isInterrupted: _state.isInterrupted,
              captureDiagnostics: diagnostics,
              error: _state.error,
            ),
          );
          if (kDebugMode) {
            debugPrint(
              '[HuddleCaptureDiagnostics] ${diagnostics.debugSummary}',
            );
          }
        } catch (_) {
          _reportMalformedNativeEvent();
        }
      default:
        return null;
    }
    return null;
  }

  void _reportMalformedNativeEvent() {
    _fail(
      const HuddleMediaError(
        code: HuddleMediaErrorCode.malformedNativeEvent,
        message: 'Native Huddle media emitted a malformed Opus frame.',
      ),
    );
  }

  void _ensureNotDisposed() {
    if (_disposed) {
      throw const HuddleMediaError(
        code: HuddleMediaErrorCode.disposed,
        message: 'Huddle media has been disposed.',
      );
    }
  }

  HuddleMediaError _platformFailure(PlatformException error) =>
      HuddleMediaError(
        code: HuddleMediaErrorCode.platformFailure,
        message: error.message ?? 'Native Huddle media failed.',
        cause: error,
      );

  void _fail(HuddleMediaError error) {
    _emit(
      HuddleMediaState(
        phase: HuddleMediaPhase.failed,
        capabilities: _capabilities,
        isMuted: _state.isMuted,
        isSpeakerEnabled: _state.isSpeakerEnabled,
        isInterrupted: _state.isInterrupted,
        captureDiagnostics: _state.captureDiagnostics,
        error: error,
      ),
    );
  }

  void _emit(HuddleMediaState next) {
    _state = next;
    if (!_stateController.isClosed) _stateController.add(next);
  }
}
