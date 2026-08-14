import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:audio_session/audio_session.dart';
import 'package:flutter_pcm_sound/flutter_pcm_sound.dart';
import 'package:opus_codec/opus_codec.dart' as opus_codec;
import 'package:opus_codec_dart/opus_codec_dart.dart';
import 'package:opus_codec_dart/wrappers/opus_defines.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:record/record.dart' hide IosAudioCategory;

import 'huddle_wire.dart';

enum HuddleOutputRoute { system, speaker, earpiece, wired, bluetooth }

class HuddleAudioCapabilityException implements Exception {
  const HuddleAudioCapabilityException(this.message);
  final String message;
  @override
  String toString() => message;
}

abstract interface class HuddleAudioEngine {
  Stream<Int16List> get microphonePcm;
  Stream<void> get interruptions;
  Future<void> start();
  Future<void> pauseMicrophone();
  Future<void> resumeMicrophone();
  Future<void> setOutputRoute(HuddleOutputRoute route);
  Future<Uint8List> encode(Int16List pcm);
  Future<Int16List> decode(int peerIndex, Uint8List opus, {bool plc = false});
  void removePeer(int peerIndex);
  Future<void> play(Int16List mixedPcm);
  Future<void> stop();
}

/// Native, full-duplex Huddle audio for Android and iOS.
///
/// Codec and plugin objects remain behind [HuddleAudioEngine], so controller
/// tests never need a microphone, platform channel, or native libopus binary.
class MobileHuddleAudioEngine implements HuddleAudioEngine {
  final AudioRecorder _recorder = AudioRecorder();
  final _microphone = StreamController<Int16List>.broadcast();
  final _interruptions = StreamController<void>.broadcast();
  final Map<int, SimpleOpusDecoder> _decoders = {};
  BufferedOpusEncoder? _encoder;
  AudioSession? _session;
  StreamSubscription<Uint8List>? _recordingSubscription;
  StreamSubscription<AudioInterruptionEvent>? _interruptionSubscription;
  Future<void> _playbackQueue = Future<void>.value();
  int? _pendingPcmByte;
  bool _started = false;
  bool _stopped = false;
  bool _microphonePaused = false;
  HuddleOutputRoute _outputRoute = HuddleOutputRoute.system;

  @override
  Stream<Int16List> get microphonePcm => _microphone.stream;

  @override
  Stream<void> get interruptions => _interruptions.stream;

  @override
  Future<void> start() async {
    if (_started) return;
    if (_stopped) throw StateError('Huddle audio engine cannot be restarted');
    if (!Platform.isAndroid && !Platform.isIOS) {
      throw const HuddleAudioCapabilityException(
        'Huddle audio requires an Android or iOS device.',
      );
    }
    if (!await _recorder.hasPermission()) {
      throw const HuddleAudioCapabilityException(
        'Microphone permission is required to join a Huddle.',
      );
    }

    try {
      initOpus(await opus_codec.load());
      final encoder = BufferedOpusEncoder(
        sampleRate: huddleSampleRate,
        channels: 1,
        application: Application.voip,
        maxInputBufferSizeBytes: huddleFrameSamples * 2,
        maxOutputBufferSizeBytes: 400,
      );
      encoder
        ..encoderCtl(request: OPUS_SET_BITRATE_REQUEST, value: 32000)
        ..encoderCtl(request: OPUS_SET_INBAND_FEC_REQUEST, value: 1)
        ..encoderCtl(request: OPUS_SET_PACKET_LOSS_PERC_REQUEST, value: 10)
        ..encoderCtl(request: OPUS_SET_DTX_REQUEST, value: 1);
      _encoder = encoder;

      await FlutterPcmSound.setLogLevel(LogLevel.none);
      await FlutterPcmSound.setup(
        sampleRate: huddleSampleRate,
        channelCount: 1,
        iosAudioCategory: IosAudioCategory.playAndRecord,
      );
      await FlutterPcmSound.setFeedThreshold(huddleFrameSamples * 2);
      FlutterPcmSound.setFeedCallback((_) {});

      // Configure after every audio plugin is loaded. AVAudioSession is global
      // and plugins may otherwise replace voice-chat mode with their defaults.
      final session = await AudioSession.instance;
      await session.configure(_voiceSessionConfiguration());
      if (!await session.setActive(true)) {
        throw const HuddleAudioCapabilityException(
          'Another app currently owns the speech audio session.',
        );
      }
      _session = session;
      await setOutputRoute(HuddleOutputRoute.system);
      _interruptionSubscription = session.interruptionEventStream.listen((
        event,
      ) {
        if (event.begin) _interruptions.add(null);
      });
      FlutterPcmSound.start();

      await _startMicrophoneStream();
      _started = true;
    } catch (error) {
      await stop();
      if (error is HuddleAudioCapabilityException) rethrow;
      throw HuddleAudioCapabilityException(
        'Could not initialize Huddle audio: $error',
      );
    }
  }

  AudioSessionConfiguration _voiceSessionConfiguration() =>
      AudioSessionConfiguration(
        avAudioSessionCategory: AVAudioSessionCategory.playAndRecord,
        avAudioSessionCategoryOptions:
            AVAudioSessionCategoryOptions.allowBluetooth |
            AVAudioSessionCategoryOptions.allowBluetoothA2dp,
        avAudioSessionMode: AVAudioSessionMode.voiceChat,
        androidAudioAttributes: const AndroidAudioAttributes(
          contentType: AndroidAudioContentType.speech,
          usage: AndroidAudioUsage.voiceCommunication,
        ),
        androidAudioFocusGainType: AndroidAudioFocusGainType.gain,
        androidWillPauseWhenDucked: true,
      );

  Future<void> _startMicrophoneStream() async {
    await _recorder.ios?.manageAudioSession(false);
    final bytes = await _recorder.startStream(
      const RecordConfig(
        encoder: AudioEncoder.pcm16bits,
        sampleRate: huddleSampleRate,
        numChannels: 1,
        autoGain: true,
        echoCancel: true,
        noiseSuppress: true,
        streamBufferSize: huddleFrameSamples * 4,
        androidConfig: AndroidRecordConfig(
          audioSource: AndroidAudioSource.voiceCommunication,
          audioManagerMode: AudioManagerMode.modeInCommunication,
        ),
      ),
    );
    _pendingPcmByte = null;
    _recordingSubscription = bytes.listen((chunk) {
      final normalized = _withPendingPcmByte(chunk);
      if (normalized.isEmpty) return;
      final data = ByteData.sublistView(normalized);
      final pcm = Int16List(normalized.length ~/ 2);
      for (var i = 0; i < pcm.length; i++) {
        pcm[i] = data.getInt16(i * 2, Endian.little);
      }
      _microphone.add(pcm);
    }, onError: _microphone.addError);
    _microphonePaused = false;
  }

  @override
  Future<void> pauseMicrophone() async {
    if (!_started || _stopped || _microphonePaused) return;
    await _recordingSubscription?.cancel();
    _recordingSubscription = null;
    await _recorder.stop();
    _pendingPcmByte = null;
    _microphonePaused = true;
  }

  @override
  Future<void> resumeMicrophone() async {
    if (!_started || _stopped || !_microphonePaused) return;
    final session = _session;
    if (session == null) return;
    await session.configure(_voiceSessionConfiguration());
    if (!await session.setActive(true)) {
      throw const HuddleAudioCapabilityException(
        'Could not restore the Huddle audio session after speech recognition.',
      );
    }
    await setOutputRoute(_outputRoute);
    await _startMicrophoneStream();
  }

  @override
  Future<Uint8List> encode(Int16List pcm) async {
    if (pcm.length != huddleFrameSamples || _encoder == null) {
      throw const HuddleAudioCapabilityException(
        'Huddle encoder is not ready.',
      );
    }
    final encoder = _encoder!;
    final input = ByteData.sublistView(encoder.inputBuffer);
    for (var i = 0; i < pcm.length; i++) {
      input.setInt16(i * 2, pcm[i], Endian.little);
    }
    encoder.inputBufferIndex = pcm.length * 2;
    return encoder.encode();
  }

  @override
  Future<Int16List> decode(
    int peerIndex,
    Uint8List opus, {
    bool plc = false,
  }) async {
    final decoder = _decoders.putIfAbsent(
      peerIndex,
      () => SimpleOpusDecoder(sampleRate: huddleSampleRate, channels: 1),
    );
    return decoder.decode(input: plc ? null : opus, loss: plc ? 20 : null);
  }

  @override
  void removePeer(int peerIndex) => _decoders.remove(peerIndex)?.destroy();

  @override
  Future<void> play(Int16List mixedPcm) {
    if (_stopped || mixedPcm.isEmpty) return Future<void>.value();
    _playbackQueue = _playbackQueue.then((_) async {
      if (!_stopped) {
        await FlutterPcmSound.feed(PcmArrayInt16.fromList(mixedPcm));
      }
    });
    return _playbackQueue;
  }

  @override
  Future<void> setOutputRoute(HuddleOutputRoute route) async {
    if (Platform.isIOS) {
      final av = AVAudioSession();
      await av.overrideOutputAudioPort(
        route == HuddleOutputRoute.speaker
            ? AVAudioSessionPortOverride.speaker
            : AVAudioSessionPortOverride.none,
      );
      _outputRoute = route;
      return;
    }
    if (Platform.isAndroid) {
      final manager = AndroidAudioManager();
      await manager.setMode(AndroidAudioHardwareMode.inCommunication);
      await manager.setSpeakerphoneOn(route == HuddleOutputRoute.speaker);
      if (route == HuddleOutputRoute.bluetooth) {
        final permission = await Permission.bluetoothConnect.request();
        if (!permission.isGranted) {
          throw const HuddleAudioCapabilityException(
            'Nearby devices permission is required to use Bluetooth audio.',
          );
        }
        await manager.startBluetoothSco();
        await manager.setBluetoothScoOn(true);
      } else {
        await manager.setBluetoothScoOn(false);
        await manager.stopBluetoothSco();
      }
      _outputRoute = route;
    }
  }

  @override
  Future<void> stop() async {
    if (_stopped) return;
    _stopped = true;
    _started = false;
    await _recordingSubscription?.cancel();
    _recordingSubscription = null;
    try {
      await _recorder.stop();
    } catch (_) {}
    await _recorder.dispose();
    await _interruptionSubscription?.cancel();
    _interruptionSubscription = null;
    try {
      await _playbackQueue;
    } catch (_) {}
    FlutterPcmSound.setFeedCallback(null);
    try {
      await FlutterPcmSound.release();
    } catch (_) {}
    _encoder?.destroy();
    _encoder = null;
    for (final decoder in _decoders.values) {
      decoder.destroy();
    }
    _decoders.clear();
    try {
      await _session?.setActive(false);
    } catch (_) {}
    _session = null;
    await _microphone.close();
    await _interruptions.close();
  }

  Uint8List _withPendingPcmByte(Uint8List chunk) {
    final pending = _pendingPcmByte;
    final combined = pending == null
        ? Uint8List.fromList(chunk)
        : Uint8List.fromList([pending, ...chunk]);
    if (combined.length.isOdd) {
      _pendingPcmByte = combined.last;
      return Uint8List.sublistView(combined, 0, combined.length - 1);
    }
    _pendingPcmByte = null;
    return combined;
  }
}
