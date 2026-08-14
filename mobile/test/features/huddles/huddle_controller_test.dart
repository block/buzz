import 'dart:async';
import 'dart:typed_data';

import 'package:buzz/features/huddles/huddle_audio.dart';
import 'package:buzz/features/huddles/huddle_bot_voice.dart';
import 'package:buzz/features/huddles/huddle_controller.dart';
import 'package:buzz/features/huddles/huddle_transport.dart';
import 'package:buzz/features/huddles/huddle_wire.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test('join, mute, downlink, route, and leave preserve boundaries', () async {
    final transport = _FakeTransport();
    final audio = _FakeAudio();
    final container = _container(transport: transport, audio: audio);
    addTearDown(container.dispose);
    final controller = container.read(huddleControllerProvider.notifier);

    await controller.join(parentChannelId: 'parent', channelId: 'huddle');
    expect(
      container.read(huddleControllerProvider).phase,
      HuddlePhase.connected,
    );
    expect(transport.channelId, 'huddle');
    expect(transport.parentChannelId, 'parent');

    controller.toggleMute();
    audio.microphone.add(Int16List(960));
    await Future<void>.delayed(Duration.zero);
    expect(transport.sentAudio, isEmpty);

    transport.eventsController.add(
      HuddleAudioFrameEvent(
        peerIndex: 7,
        bytes: Uint8List.fromList([
          ...const HuddleFrameHeader(
            sequence: 1,
            timestamp48k: 960,
            levelDbov: -20,
            flags: 0,
          ).encode(),
          9,
        ]),
      ),
    );
    await Future<void>.delayed(const Duration(milliseconds: 25));
    expect(audio.decodedPeers, [7]);
    expect(audio.played, hasLength(1));

    transport.eventsController.add(
      HuddleAudioFrameEvent(
        peerIndex: 7,
        bytes: Uint8List.fromList([
          ...const HuddleFrameHeader(
            sequence: 3,
            timestamp48k: 2880,
            levelDbov: -20,
            flags: 0,
          ).encode(),
          10,
        ]),
      ),
    );
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(audio.plcCalls, 1);
    expect(audio.played, hasLength(3));

    await controller.setOutputRoute(HuddleOutputRoute.speaker);
    expect(audio.route, HuddleOutputRoute.speaker);
    await controller.leave();
    expect(container.read(huddleControllerProvider).phase, HuddlePhase.idle);
    expect(transport.sentLeave, isTrue);
    expect(audio.stopped, isTrue);
  });

  test('capability failure is visible and tears down', () async {
    final transport = _FakeTransport();
    final audio = _FakeAudio(
      startError: const HuddleAudioCapabilityException('denied'),
    );
    final container = _container(transport: transport, audio: audio);
    addTearDown(container.dispose);

    await expectLater(
      container
          .read(huddleControllerProvider.notifier)
          .join(parentChannelId: 'parent', channelId: 'huddle'),
      throwsA(isA<HuddleAudioCapabilityException>()),
    );
    final state = container.read(huddleControllerProvider);
    expect(state.phase, HuddlePhase.failed);
    expect(state.error, contains('denied'));
  });

  test('audio interruption leaves without creating a second session', () async {
    final transport = _FakeTransport();
    final audio = _FakeAudio();
    final container = _container(transport: transport, audio: audio);
    addTearDown(container.dispose);
    await container
        .read(huddleControllerProvider.notifier)
        .join(parentChannelId: 'parent', channelId: 'huddle');
    audio.interrupted.add(null);
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);
    expect(container.read(huddleControllerProvider).phase, HuddlePhase.idle);
    expect(audio.starts, 1);
  });

  test('route permission failure is visible and preserves the route', () async {
    final transport = _FakeTransport();
    final audio = _FakeAudio(routeError: StateError('permission denied'));
    final container = _container(transport: transport, audio: audio);
    addTearDown(container.dispose);
    final controller = container.read(huddleControllerProvider.notifier);
    await controller.join(parentChannelId: 'parent', channelId: 'huddle');

    await controller.setOutputRoute(HuddleOutputRoute.bluetooth);

    final state = container.read(huddleControllerProvider);
    expect(state.outputRoute, HuddleOutputRoute.system);
    expect(state.error, contains('permission denied'));
  });

  test('bot voice turn pauses audio, mentions bots, and resumes', () async {
    final transport = _FakeTransport();
    final audio = _FakeAudio();
    final botVoice = _FakeBotVoice(transcript: 'What should we build?');
    String? sentChannel;
    String? sentContent;
    List<String>? sentMentions;
    final container = _container(
      transport: transport,
      audio: audio,
      botVoice: botVoice,
      botPubkeys: const {'bot-pubkey'},
      transcriptSender: (channelId, content, mentions) async {
        sentChannel = channelId;
        sentContent = content;
        sentMentions = mentions;
      },
    );
    addTearDown(container.dispose);
    final controller = container.read(huddleControllerProvider.notifier);
    await controller.join(parentChannelId: 'parent', channelId: 'huddle');

    await controller.talkToBots();

    expect(audio.pauses, 1);
    expect(audio.resumes, 1);
    expect(sentChannel, 'huddle');
    expect(sentContent, 'What should we build?');
    expect(sentMentions, ['bot-pubkey']);
    expect(
      container.read(huddleControllerProvider).lastTranscript,
      sentContent,
    );
  });

  test(
    'leave during bot discovery cannot reactivate a joined Huddle',
    () async {
      final transport = _FakeTransport();
      final audio = _FakeAudio();
      final discoveryStarted = Completer<void>();
      final releaseDiscovery = Completer<Set<String>>();
      final container = _container(
        transport: transport,
        audio: audio,
        botPubkeysLoader: (_) {
          discoveryStarted.complete();
          return releaseDiscovery.future;
        },
      );
      addTearDown(container.dispose);
      final controller = container.read(huddleControllerProvider.notifier);

      final joining = controller.join(
        parentChannelId: 'parent',
        channelId: 'huddle',
      );
      await discoveryStarted.future;
      await controller.leave();
      releaseDiscovery.complete(const {'bot-pubkey'});
      await joining;

      expect(container.read(huddleControllerProvider).phase, HuddlePhase.idle);
      expect(audio.stopped, isTrue);
    },
  );

  test('leave during bot subscription immediately unsubscribes', () async {
    final transport = _FakeTransport();
    final audio = _FakeAudio();
    final subscriptionStarted = Completer<void>();
    final releaseSubscription = Completer<void>();
    var unsubscribes = 0;
    final container = _container(
      transport: transport,
      audio: audio,
      botPubkeys: const {'bot-pubkey'},
      messageSubscriber: (_, _) async {
        subscriptionStarted.complete();
        await releaseSubscription.future;
        return () => unsubscribes++;
      },
    );
    addTearDown(container.dispose);
    final controller = container.read(huddleControllerProvider.notifier);

    final joining = controller.join(
      parentChannelId: 'parent',
      channelId: 'huddle',
    );
    await subscriptionStarted.future;
    await controller.leave();
    releaseSubscription.complete();
    await joining;

    expect(container.read(huddleControllerProvider).phase, HuddlePhase.idle);
    expect(unsubscribes, 1);
  });
}

ProviderContainer _container({
  required _FakeTransport transport,
  required _FakeAudio audio,
  _FakeBotVoice? botVoice,
  Set<String> botPubkeys = const {},
  HuddleBotPubkeysLoader? botPubkeysLoader,
  HuddleMessageSubscriber? messageSubscriber,
  HuddleTranscriptSender? transcriptSender,
}) {
  return ProviderContainer(
    overrides: [
      relayConfigProvider.overrideWith(() => _RelayConfig()),
      huddleControllerProvider.overrideWith(
        () => HuddleController(
          transportFactory: () => transport,
          audioFactory: () => audio,
          botVoiceFactory: () => botVoice ?? _FakeBotVoice(),
          botPubkeysLoader: botPubkeysLoader ?? (_) async => botPubkeys,
          messageSubscriber: messageSubscriber ?? (_, _) async => () {},
          transcriptSender: transcriptSender,
        ),
      ),
    ],
  );
}

class _RelayConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(
    baseUrl: 'wss://relay.example',
    nsec:
        'nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqc824v',
  );
}

class _FakeTransport implements HuddleTransport {
  final eventsController = StreamController<HuddleTransportEvent>.broadcast();
  final sentAudio = <Uint8List>[];
  String? channelId;
  String? parentChannelId;
  bool sentLeave = false;

  @override
  Stream<HuddleTransportEvent> get events => eventsController.stream;
  @override
  Future<void> connect({
    required Uri relayWebSocket,
    required String channelId,
    required String? parentChannelId,
    required String nsec,
  }) async {
    this.channelId = channelId;
    this.parentChannelId = parentChannelId;
  }

  @override
  void sendAudio(Uint8List bytes) => sentAudio.add(bytes);
  @override
  Future<void> close({bool sendLeave = true}) async => sentLeave = sendLeave;
}

class _FakeAudio implements HuddleAudioEngine {
  _FakeAudio({this.startError, this.routeError});
  final Object? startError;
  final Object? routeError;
  final microphone = StreamController<Int16List>.broadcast();
  final interrupted = StreamController<void>.broadcast();
  final decodedPeers = <int>[];
  final played = <Int16List>[];
  int starts = 0;
  int pauses = 0;
  int resumes = 0;
  int plcCalls = 0;
  bool stopped = false;
  HuddleOutputRoute? route;

  @override
  Stream<void> get interruptions => interrupted.stream;
  @override
  Stream<Int16List> get microphonePcm => microphone.stream;
  @override
  Future<void> start() async {
    starts++;
    if (startError != null) throw startError!;
  }

  @override
  Future<void> pauseMicrophone() async => pauses++;

  @override
  Future<void> resumeMicrophone() async => resumes++;

  @override
  Future<Uint8List> encode(Int16List pcm) async =>
      Uint8List.fromList([1, 2, 3]);
  @override
  Future<Int16List> decode(
    int peerIndex,
    Uint8List opus, {
    bool plc = false,
  }) async {
    decodedPeers.add(peerIndex);
    if (plc) plcCalls++;
    return Int16List(960);
  }

  @override
  void removePeer(int peerIndex) {}

  @override
  Future<void> play(Int16List mixedPcm) async => played.add(mixedPcm);
  @override
  Future<void> setOutputRoute(HuddleOutputRoute route) async {
    if (routeError != null) throw routeError!;
    this.route = route;
  }

  @override
  Future<void> stop() async => stopped = true;
}

class _FakeBotVoice implements HuddleBotVoiceEngine {
  _FakeBotVoice({this.transcript});

  final String? transcript;
  final spoken = <String>[];

  @override
  Future<String?> transcribe() async => transcript;

  @override
  Future<void> stopTranscribing() async {}

  @override
  Future<void> speak(String text) async => spoken.add(text);

  @override
  Future<void> stopSpeaking() async {}

  @override
  Future<void> dispose() async {}
}
