import 'dart:async';
import 'dart:collection';
import 'dart:typed_data';

import 'package:buzz/shared/huddle/huddle.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _privateKey =
    '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';
const _parentChannelId = '11111111-2222-4333-8444-555555555555';
const _ephemeralChannelId = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';

void main() {
  test('joins unmuted and bridges remote and local Opus frames', () async {
    final media = _FakeMedia();
    final transport = _FakeTransport();
    final container = ProviderContainer(
      overrides: [
        huddleMediaFactoryProvider.overrideWithValue(() => media),
        huddleTransportFactoryProvider.overrideWithValue((_) => transport),
      ],
    );
    addTearDown(container.dispose);
    final controller = container.read(huddleSessionProvider.notifier);

    await controller.join(
      _parameters(),
      currentPubkey: 'mobile',
      isCreator: true,
      startedEventId: 'start-event',
    );

    expect(
      container.read(huddleSessionProvider).phase,
      HuddleSessionPhase.connected,
    );
    expect(container.read(huddleSessionProvider).isMuted, isFalse);
    expect(container.read(huddleSessionProvider).participantCount, 2);
    expect(container.read(huddleSessionProvider).participantPubkeys, [
      'desktop',
      'mobile',
    ]);
    expect(container.read(huddleSessionProvider).isCreator, isTrue);

    final remote = HuddleRemoteAudioFrame(
      peerIndex: 1,
      epoch: 0,
      header: const HuddleAudioHeader(
        sequence: 3,
        timestamp48k: 960,
        levelDbov: -30,
        flags: 0,
      ),
      opusPayload: Uint8List.fromList([1, 2, 3]),
    );
    transport.emitRemote(remote);
    await Future<void>.delayed(Duration.zero);
    expect(media.playedFrames, [remote]);
    expect(container.read(huddleSessionProvider).receivedFrameCount, 1);
    expect(
      container.read(huddleSessionProvider).activeSpeakerPubkeys,
      contains('desktop'),
    );
    expect(
      container.read(huddleSessionProvider).speakerLevels['desktop'],
      closeTo(25 / 55, 0.001),
    );

    media.emitLocal(
      HuddleLocalAudioFrame(
        header: const HuddleAudioHeader(
          sequence: 4,
          timestamp48k: 1920,
          levelDbov: -20,
          flags: 0,
        ),
        opusPayload: Uint8List.fromList([4, 5]),
      ),
    );

    expect(transport.sentFrames.single.opusPayload, [4, 5]);
    expect(container.read(huddleSessionProvider).sentFrameCount, 1);
    expect(container.read(huddleSessionProvider).isMuted, isFalse);
    expect(
      container.read(huddleSessionProvider).speakerLevels['mobile'],
      closeTo(35 / 55, 0.001),
    );

    await controller.setMuted(true);
    media.emitLocal(
      HuddleLocalAudioFrame(
        header: const HuddleAudioHeader(
          sequence: 5,
          timestamp48k: 2880,
          levelDbov: -20,
          flags: 0,
        ),
        opusPayload: Uint8List.fromList([6, 7]),
      ),
    );
    expect(transport.sentFrames, hasLength(1));
    expect(container.read(huddleSessionProvider).isMuted, isTrue);

    await controller.setSpeakerEnabled(true);
    expect(container.read(huddleSessionProvider).isSpeakerEnabled, isTrue);
    expect(media.state.isSpeakerEnabled, isTrue);
    await controller.leave();
  });

  test('bounds playback per peer, drops oldest, and drains fairly', () async {
    final media = _FakeMedia(blockPlayback: true);
    final transport = _FakeTransport();
    final container = ProviderContainer(
      overrides: [
        huddleMediaFactoryProvider.overrideWithValue(() => media),
        huddleTransportFactoryProvider.overrideWithValue((_) => transport),
      ],
    );
    addTearDown(container.dispose);
    await container.read(huddleSessionProvider.notifier).join(_parameters());

    for (var sequence = 0; sequence < 20; sequence++) {
      transport.emitRemote(_remoteFrame(peerIndex: 1, sequence: sequence));
    }
    transport.emitRemote(_remoteFrame(peerIndex: 2, sequence: 100));
    await Future<void>.delayed(Duration.zero);
    expect(media.playedFrames.single.header.sequence, 0);

    media.releaseNextPlayback();
    await _waitUntil(() => media.playedFrames.length == 2);
    expect(media.playedFrames[1].peerIndex, 2);
    media.releaseNextPlayback();
    await _waitUntil(() => media.playedFrames.length == 3);
    expect(media.playedFrames[2].header.sequence, 10);
  });

  test('peer replacement clears queued and native playback first', () async {
    final media = _FakeMedia(blockPlayback: true);
    final transport = _FakeTransport();
    final container = ProviderContainer(
      overrides: [
        huddleMediaFactoryProvider.overrideWithValue(() => media),
        huddleTransportFactoryProvider.overrideWithValue((_) => transport),
      ],
    );
    addTearDown(container.dispose);
    await container.read(huddleSessionProvider.notifier).join(_parameters());

    transport.emitRemote(_remoteFrame(peerIndex: 1, sequence: 1));
    transport.emitRemote(_remoteFrame(peerIndex: 1, sequence: 2));
    transport.emitReplacement(
      const HuddlePeer(pubkey: 'desktop', peerIndex: 1, epoch: 0),
      const HuddlePeer(pubkey: 'new', peerIndex: 1, epoch: 1),
    );
    await Future<void>.delayed(Duration.zero);
    expect(media.removedPeers, [1]);

    media.releaseNextPlayback();
    await Future<void>.delayed(Duration.zero);
    expect(media.playedFrames.map((frame) => frame.header.sequence), [1]);
  });

  test('keeps media alive through a bounded transport reconnect', () async {
    final media = _FakeMedia();
    final transport = _FakeTransport();
    final container = ProviderContainer(
      overrides: [
        huddleMediaFactoryProvider.overrideWithValue(() => media),
        huddleTransportFactoryProvider.overrideWithValue((_) => transport),
        huddleReconnectDelaysProvider.overrideWithValue(const [Duration.zero]),
      ],
    );
    addTearDown(container.dispose);

    await container.read(huddleSessionProvider.notifier).join(_parameters());
    transport.emitUnexpectedFailure();
    await _waitUntil(() => transport.connectCalls == 2);

    expect(
      container.read(huddleSessionProvider).phase,
      HuddleSessionPhase.connected,
    );
    expect(media.startCalls, 1);
  });

  test('surfaces native audio interruptions without leaving', () async {
    final media = _FakeMedia();
    final transport = _FakeTransport();
    final container = ProviderContainer(
      overrides: [
        huddleMediaFactoryProvider.overrideWithValue(() => media),
        huddleTransportFactoryProvider.overrideWithValue((_) => transport),
      ],
    );
    addTearDown(container.dispose);

    await container.read(huddleSessionProvider.notifier).join(_parameters());
    media.emitInterrupted(true);
    expect(
      container.read(huddleSessionProvider).phase,
      HuddleSessionPhase.interrupted,
    );
    media.emitInterrupted(false);
    expect(
      container.read(huddleSessionProvider).phase,
      HuddleSessionPhase.connected,
    );
  });

  test('permission denial is an explicit failed state', () async {
    final media = _FakeMedia(permission: HuddleMicrophonePermission.denied);
    final transport = _FakeTransport();
    final container = ProviderContainer(
      overrides: [
        huddleMediaFactoryProvider.overrideWithValue(() => media),
        huddleTransportFactoryProvider.overrideWithValue((_) => transport),
      ],
    );
    addTearDown(container.dispose);

    await container.read(huddleSessionProvider.notifier).join(_parameters());

    final state = container.read(huddleSessionProvider);
    expect(state.phase, HuddleSessionPhase.failed);
    expect(state.error, contains('Microphone permission'));
    expect(state.microphonePermissionRequired, isTrue);
    expect(state.wasAdmitted, isFalse);
    expect(transport.connectCalls, 0);
  });

  test(
    'openMicrophoneSettings delegates to the media settings launcher',
    () async {
      final media = _FakeMedia(permission: HuddleMicrophonePermission.denied);
      final transport = _FakeTransport();
      final container = ProviderContainer(
        overrides: [
          huddleMediaFactoryProvider.overrideWithValue(() => media),
          huddleTransportFactoryProvider.overrideWithValue((_) => transport),
        ],
      );
      addTearDown(container.dispose);

      await container.read(huddleSessionProvider.notifier).join(_parameters());
      // The failed join disposed its media, so recovery uses a transient one.
      final notifier = container.read(huddleSessionProvider.notifier);
      expect(await notifier.openMicrophoneSettings(), isTrue);
    },
  );

  test(
    'native failure releases media and transport before publishing failure',
    () async {
      final release = Completer<void>();
      final media = _FakeMedia(disposeGate: release.future);
      final transport = _FakeTransport();
      final container = ProviderContainer(
        overrides: [
          huddleMediaFactoryProvider.overrideWithValue(() => media),
          huddleTransportFactoryProvider.overrideWithValue((_) => transport),
        ],
      );
      addTearDown(container.dispose);

      await container.read(huddleSessionProvider.notifier).join(_parameters());
      media.emitFailure();
      await Future<void>.delayed(Duration.zero);

      expect(media.disposeCalls, 1);
      expect(transport.disposeCalls, 0);
      expect(
        container.read(huddleSessionProvider).phase,
        HuddleSessionPhase.connected,
      );

      release.complete();
      await _waitUntil(
        () =>
            container.read(huddleSessionProvider).phase ==
            HuddleSessionPhase.failed,
      );
      expect(transport.disposeCalls, 1);
    },
  );

  test(
    'retains admission evidence after an established transport fails',
    () async {
      final media = _FakeMedia();
      final transport = _FakeTransport();
      final container = ProviderContainer(
        overrides: [
          huddleMediaFactoryProvider.overrideWithValue(() => media),
          huddleTransportFactoryProvider.overrideWithValue((_) => transport),
          huddleReconnectDelaysProvider.overrideWithValue(const []),
        ],
      );
      addTearDown(container.dispose);

      await container
          .read(huddleSessionProvider.notifier)
          .join(_parameters(), currentPubkey: 'mobile');
      expect(container.read(huddleSessionProvider).wasAdmitted, isTrue);

      transport.emitUnexpectedFailure();
      await _waitUntil(
        () =>
            container.read(huddleSessionProvider).phase ==
            HuddleSessionPhase.failed,
      );

      expect(container.read(huddleSessionProvider).wasAdmitted, isTrue);
    },
  );
}

HuddleRemoteAudioFrame _remoteFrame({
  required int peerIndex,
  required int sequence,
  int epoch = 0,
}) => HuddleRemoteAudioFrame(
  peerIndex: peerIndex,
  epoch: epoch,
  header: HuddleAudioHeader(
    sequence: sequence,
    timestamp48k: sequence * 960,
    levelDbov: -20,
    flags: 0,
  ),
  opusPayload: Uint8List.fromList([sequence & 0xff]),
);

HuddleConnectionParameters _parameters() => HuddleConnectionParameters(
  relayWebSocketUrl: 'wss://buzz.example',
  nsec: _privateKey,
  parentChannelId: _parentChannelId,
  ephemeralChannelId: _ephemeralChannelId,
);

final class _FakeMedia implements HuddleMedia {
  _FakeMedia({
    this.permission = HuddleMicrophonePermission.granted,
    this.blockPlayback = false,
    this.disposeGate,
  });

  final HuddleMicrophonePermission permission;
  final bool blockPlayback;
  final Future<void>? disposeGate;
  final _states = StreamController<HuddleMediaState>.broadcast(sync: true);
  final _localFrames = StreamController<HuddleLocalAudioFrame>.broadcast(
    sync: true,
  );
  final List<HuddleRemoteAudioFrame> playedFrames = [];
  final List<int> removedPeers = [];
  final Queue<Completer<void>> _playbackCompleters = Queue();
  HuddleMediaState _state = const HuddleMediaState(
    phase: HuddleMediaPhase.idle,
  );
  var startCalls = 0;
  var disposeCalls = 0;

  void emitLocal(HuddleLocalAudioFrame frame) => _localFrames.add(frame);

  void emitInterrupted(bool interrupted) {
    _state = HuddleMediaState(
      phase: HuddleMediaPhase.active,
      capabilities: _state.capabilities,
      isMuted: _state.isMuted,
      isSpeakerEnabled: _state.isSpeakerEnabled,
      isInterrupted: interrupted,
    );
    _states.add(_state);
  }

  void emitFailure() {
    _state = HuddleMediaState(
      phase: HuddleMediaPhase.failed,
      capabilities: _state.capabilities,
      error: const HuddleMediaError(
        code: HuddleMediaErrorCode.platformFailure,
        message: 'native playback failed',
      ),
    );
    _states.add(_state);
  }

  @override
  HuddleMediaState get state => _state;

  @override
  Stream<HuddleMediaState> get states => _states.stream;

  @override
  Stream<HuddleLocalAudioFrame> get localAudioFrames => _localFrames.stream;

  @override
  Future<HuddleMediaCapabilities> discoverCapabilities() async {
    const capabilities = HuddleMediaCapabilities(
      platform: 'test',
      supportsAudioSession: true,
      supportsMicrophonePermission: true,
      supportsCapture: true,
      supportsPlayback: true,
      supportsOpusEncoding: true,
      supportsOpusDecoding: true,
    );
    _state = const HuddleMediaState(
      phase: HuddleMediaPhase.idle,
      capabilities: capabilities,
    );
    return capabilities;
  }

  @override
  Future<HuddleMicrophonePermission> requestMicrophonePermission() async =>
      permission;

  var openSettingsCalls = 0;
  bool openSettingsResult = true;

  @override
  Future<bool> openSystemSettings() async {
    openSettingsCalls += 1;
    return openSettingsResult;
  }

  @override
  Future<void> prepare() async {
    _state = HuddleMediaState(
      phase: HuddleMediaPhase.prepared,
      capabilities: _state.capabilities,
    );
    _states.add(_state);
  }

  @override
  Future<void> start() async {
    startCalls += 1;
    _state = HuddleMediaState(
      phase: HuddleMediaPhase.active,
      capabilities: _state.capabilities,
    );
    _states.add(_state);
  }

  @override
  Future<void> setMuted(bool muted) async {
    _state = HuddleMediaState(
      phase: HuddleMediaPhase.active,
      capabilities: _state.capabilities,
      isMuted: muted,
    );
    _states.add(_state);
  }

  @override
  Future<void> setSpeakerEnabled(bool enabled) async {
    _state = HuddleMediaState(
      phase: HuddleMediaPhase.active,
      capabilities: _state.capabilities,
      isMuted: _state.isMuted,
      isSpeakerEnabled: enabled,
    );
    _states.add(_state);
  }

  @override
  Future<void> playRemoteFrame(HuddleRemoteAudioFrame frame) async {
    playedFrames.add(frame);
    if (blockPlayback) {
      final completer = Completer<void>();
      _playbackCompleters.addLast(completer);
      await completer.future;
    }
  }

  void releaseNextPlayback() => _playbackCompleters.removeFirst().complete();

  @override
  Future<void> removeRemotePeer(int peerIndex) async {
    removedPeers.add(peerIndex);
  }

  @override
  Future<void> stop() async {
    _state = HuddleMediaState(
      phase: HuddleMediaPhase.stopped,
      capabilities: _state.capabilities,
    );
    _states.add(_state);
  }

  @override
  Future<void> dispose() async {
    disposeCalls += 1;
    if (disposeGate case final gate?) await gate;
    await stop();
  }
}

final class _FakeTransport implements HuddleTransportClient {
  final _states = StreamController<HuddleTransportState>.broadcast(sync: true);
  final _remoteFrames = StreamController<HuddleRemoteAudioFrame>.broadcast(
    sync: true,
  );
  final _peerEvents = StreamController<HuddlePeerEvent>.broadcast(sync: true);
  final _issues = StreamController<HuddleTransportError>.broadcast(sync: true);
  final List<HuddleLocalAudioFrame> sentFrames = [];
  var connectCalls = 0;
  var disposeCalls = 0;
  HuddleTransportState _state = HuddleTransportState.idle();

  void emitRemote(HuddleRemoteAudioFrame frame) => _remoteFrames.add(frame);

  void emitReplacement(HuddlePeer peer, HuddlePeer replacement) {
    _peerEvents.add(
      HuddlePeerEvent(
        type: HuddlePeerEventType.replaced,
        peer: peer,
        replacement: replacement,
      ),
    );
  }

  void emitUnexpectedFailure() {
    _state = HuddleTransportState(
      phase: HuddleTransportPhase.failed,
      peers: _state.peers,
      error: const HuddleTransportError(
        code: HuddleTransportErrorCode.socketClosed,
        message: 'socket closed',
      ),
    );
    _states.add(_state);
  }

  @override
  HuddleTransportState get state => _state;

  @override
  Stream<HuddleTransportState> get states => _states.stream;

  @override
  Stream<HuddleRemoteAudioFrame> get remoteAudioFrames => _remoteFrames.stream;

  @override
  Stream<HuddlePeerEvent> get peerEvents => _peerEvents.stream;

  @override
  Stream<HuddleTransportError> get issues => _issues.stream;

  @override
  Future<void> connect() async {
    connectCalls += 1;
    _state = HuddleTransportState(
      phase: HuddleTransportPhase.connected,
      localPeerIndex: 2,
      peers: const {
        1: HuddlePeer(pubkey: 'desktop', peerIndex: 1, epoch: 0),
        2: HuddlePeer(pubkey: 'mobile', peerIndex: 2, epoch: 0),
      },
    );
    _states.add(_state);
  }

  @override
  void sendOpusFrame({
    required HuddleAudioHeader header,
    required Uint8List opusPayload,
  }) {
    sentFrames.add(
      HuddleLocalAudioFrame(header: header, opusPayload: opusPayload),
    );
  }

  @override
  Future<void> disconnect() async {
    _state = HuddleTransportState(phase: HuddleTransportPhase.disconnected);
    _states.add(_state);
  }

  @override
  Future<void> dispose() async {
    disposeCalls += 1;
    await disconnect();
  }
}

Future<void> _waitUntil(bool Function() predicate) async {
  for (var i = 0; i < 100; i++) {
    if (predicate()) return;
    await Future<void>.delayed(Duration.zero);
  }
  fail('Timed out waiting for asynchronous Huddle state');
}
