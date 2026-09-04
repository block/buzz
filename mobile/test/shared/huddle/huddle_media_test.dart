import 'package:buzz/shared/huddle/huddle_media.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel(huddleMediaMethodChannelName);
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

  tearDown(() {
    messenger.setMockMethodCallHandler(channel, null);
  });

  test(
    'prepares the fixed v2 audio session without overstating codecs',
    () async {
      final calls = <MethodCall>[];
      messenger.setMockMethodCallHandler(channel, (call) async {
        calls.add(call);
        return switch (call.method) {
          'getCapabilities' => {
            'platform': 'android',
            'audioSession': true,
            'microphonePermission': true,
            'capture': false,
            'playback': false,
            'opusEncoding': false,
            'opusDecoding': false,
          },
          'requestMicrophonePermission' => 'granted',
          'prepare' => {'audioSessionPrepared': true},
          'stop' => null,
          _ => throw MissingPluginException(),
        };
      });
      final media = MethodChannelHuddleMedia(channel: channel);
      addTearDown(media.dispose);

      final capabilities = await media.discoverCapabilities();
      expect(capabilities.supportsAudioSession, isTrue);
      expect(capabilities.supportsFullDuplexOpus, isFalse);
      expect(
        await media.requestMicrophonePermission(),
        HuddleMicrophonePermission.granted,
      );
      await media.prepare();

      expect(media.state.phase, HuddleMediaPhase.prepared);
      final prepare = calls.singleWhere((call) => call.method == 'prepare');
      expect(prepare.arguments, {
        'protocolVersion': 2,
        'sampleRateHz': 48000,
        'channels': 1,
        'frameSamples': 960,
      });
      await expectLater(
        media.start(),
        throwsA(
          isA<HuddleMediaError>().having(
            (error) => error.code,
            'code',
            HuddleMediaErrorCode.unsupported,
          ),
        ),
      );
      expect(media.state.phase, HuddleMediaPhase.prepared);
    },
  );

  test('reports a missing native implementation as unavailable', () async {
    final media = MethodChannelHuddleMedia(channel: channel);
    addTearDown(media.dispose);

    final capabilities = await media.discoverCapabilities();

    expect(capabilities, same(HuddleMediaCapabilities.unavailable));
    expect(media.state.phase, HuddleMediaPhase.unavailable);
  });

  test('opens system settings and reports the native launch result', () async {
    var invoked = 0;
    messenger.setMockMethodCallHandler(channel, (call) async {
      invoked += 1;
      expect(call.method, 'openSystemSettings');
      return true;
    });
    final media = MethodChannelHuddleMedia(channel: channel);
    addTearDown(media.dispose);

    expect(await media.openSystemSettings(), isTrue);
    expect(invoked, 1);
  });

  test('missing native settings support degrades to not launched', () async {
    final media = MethodChannelHuddleMedia(channel: channel);
    addTearDown(media.dispose);

    expect(await media.openSystemSettings(), isFalse);
  });

  test('starts unmuted and forwards direct mute and speaker changes', () async {
    final calls = <MethodCall>[];
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      return switch (call.method) {
        'getCapabilities' => {
          'platform': 'android',
          'audioSession': true,
          'microphonePermission': true,
          'capture': true,
          'playback': true,
          'opusEncoding': true,
          'opusDecoding': true,
        },
        'prepare' => {'audioSessionPrepared': true},
        'start' || 'setMuted' || 'setSpeakerEnabled' || 'stop' => null,
        _ => throw MissingPluginException(),
      };
    });
    final media = MethodChannelHuddleMedia(channel: channel);
    addTearDown(media.dispose);

    await media.discoverCapabilities();
    await media.prepare();
    await media.start();
    expect(media.state.isMuted, isFalse);
    expect(media.state.isSpeakerEnabled, isFalse);

    await media.setMuted(true);

    expect(media.state.phase, HuddleMediaPhase.active);
    expect(media.state.isMuted, isTrue);
    expect(calls.singleWhere((call) => call.method == 'setMuted').arguments, {
      'muted': true,
    });

    await media.setSpeakerEnabled(true);

    expect(media.state.isSpeakerEnabled, isTrue);
    expect(
      calls.singleWhere((call) => call.method == 'setSpeakerEnabled').arguments,
      {'enabled': true},
    );
  });

  test('parses aggregate capture diagnostics without audio content', () {
    final diagnostics = HuddleCaptureDiagnostics.fromMap({
      'frameCount': 50,
      'rmsDbovHistogram': {'-42': 30, '-36': 20},
      'peakDbovHistogram': {'-24': 40, '-18': 10},
      'maxPeakDbov': -18,
      'audioSource': 7,
      'audioSourceLabel': 'voice_communication',
      'deviceId': 4,
      'deviceType': 1,
      'deviceLabel': 'Pixel microphone',
      'acousticEchoCancelerAvailable': true,
      'acousticEchoCancelerEnabled': true,
      'noiseSuppressorAvailable': true,
      'noiseSuppressorEnabled': true,
      'automaticGainControlAvailable': true,
      'automaticGainControlEnabled': false,
    });

    expect(diagnostics.rmsDbovHistogram, {-42: 30, -36: 20});
    expect(diagnostics.peakDbovHistogram, {-24: 40, -18: 10});
    expect(diagnostics.maxPeakDbov, -18);
    expect(diagnostics.debugSummary, contains('voice_communication'));
  });
}
