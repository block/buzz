import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_management_provider.dart';
import '../channels/send_message_provider.dart';
import 'huddle_audio.dart';
import 'huddle_bot_voice.dart';
import 'huddle_transport.dart';
import 'huddle_wire.dart';

enum HuddlePhase { idle, creating, connecting, connected, reconnecting, failed }

class HuddleState {
  const HuddleState({
    this.phase = HuddlePhase.idle,
    this.parentChannelId,
    this.channelId,
    this.isCreator = false,
    this.isMuted = false,
    this.botPubkeys = const {},
    this.isTranscribing = false,
    this.isBotSpeaking = false,
    this.lastTranscript,
    this.outputRoute = HuddleOutputRoute.system,
    this.peers = const {},
    this.error,
  });

  final HuddlePhase phase;
  final String? parentChannelId;
  final String? channelId;
  final bool isCreator;
  final bool isMuted;
  final Set<String> botPubkeys;
  final bool isTranscribing;
  final bool isBotSpeaking;
  final String? lastTranscript;
  final HuddleOutputRoute outputRoute;
  final Map<int, HuddlePeer> peers;
  final String? error;

  bool get isActive => phase != HuddlePhase.idle && phase != HuddlePhase.failed;

  HuddleState copyWith({
    HuddlePhase? phase,
    String? parentChannelId,
    String? channelId,
    bool? isCreator,
    bool? isMuted,
    Set<String>? botPubkeys,
    bool? isTranscribing,
    bool? isBotSpeaking,
    String? lastTranscript,
    HuddleOutputRoute? outputRoute,
    Map<int, HuddlePeer>? peers,
    String? error,
    bool clearError = false,
  }) => HuddleState(
    phase: phase ?? this.phase,
    parentChannelId: parentChannelId ?? this.parentChannelId,
    channelId: channelId ?? this.channelId,
    isCreator: isCreator ?? this.isCreator,
    isMuted: isMuted ?? this.isMuted,
    botPubkeys: botPubkeys ?? this.botPubkeys,
    isTranscribing: isTranscribing ?? this.isTranscribing,
    isBotSpeaking: isBotSpeaking ?? this.isBotSpeaking,
    lastTranscript: lastTranscript ?? this.lastTranscript,
    outputRoute: outputRoute ?? this.outputRoute,
    peers: peers ?? this.peers,
    error: clearError ? null : error ?? this.error,
  );
}

typedef HuddleTransportFactory = HuddleTransport Function();
typedef HuddleAudioFactory = HuddleAudioEngine Function();
typedef HuddleBotVoiceFactory = HuddleBotVoiceEngine Function();
typedef HuddleBotPubkeysLoader = Future<Set<String>> Function(String channelId);
typedef HuddleMessageSubscriber =
    Future<void Function()> Function(
      String channelId,
      void Function(NostrEvent event) onEvent,
    );
typedef HuddleTranscriptSender =
    Future<void> Function(
      String channelId,
      String content,
      List<String> mentionPubkeys,
    );

class HuddleController extends Notifier<HuddleState> {
  HuddleController({
    HuddleTransportFactory? transportFactory,
    HuddleAudioFactory? audioFactory,
    HuddleBotVoiceFactory? botVoiceFactory,
    HuddleBotPubkeysLoader? botPubkeysLoader,
    HuddleMessageSubscriber? messageSubscriber,
    HuddleTranscriptSender? transcriptSender,
  }) : _transportFactory = transportFactory ?? WebSocketHuddleTransport.new,
       _audioFactory = audioFactory ?? MobileHuddleAudioEngine.new,
       _botVoiceFactory = botVoiceFactory ?? MobileHuddleBotVoiceEngine.new,
       _botPubkeysLoader = botPubkeysLoader,
       _messageSubscriber = messageSubscriber,
       _transcriptSender = transcriptSender;

  final HuddleTransportFactory _transportFactory;
  final HuddleAudioFactory _audioFactory;
  final HuddleBotVoiceFactory _botVoiceFactory;
  final HuddleBotPubkeysLoader? _botPubkeysLoader;
  final HuddleMessageSubscriber? _messageSubscriber;
  final HuddleTranscriptSender? _transcriptSender;
  HuddleTransport? _transport;
  HuddleAudioEngine? _audio;
  HuddleBotVoiceEngine? _botVoice;
  StreamSubscription<HuddleTransportEvent>? _transportSub;
  StreamSubscription<Int16List>? _microphoneSub;
  StreamSubscription<void>? _interruptionSub;
  void Function()? _messageUnsubscribe;
  Timer? _reconnectTimer;
  Timer? _playoutTimer;
  int _generation = 0;
  int _reconnectAttempt = 0;
  Future<void> _encodeQueue = Future<void>.value();
  Future<void> _decodeQueue = Future<void>.value();
  Future<void> _speechQueue = Future<void>.value();
  final Set<String> _spokenMessageIds = {};
  final Map<int, ListQueue<Int16List>> _playoutQueues = {};
  final Map<int, int> _lastPeerSequence = {};
  final _chunker = PcmFrameChunker();
  final _sequence = HuddleSequence();

  @override
  HuddleState build() {
    ref.listen(appLifecycleProvider, (_, next) {
      if (next == AppLifecycleState.paused ||
          next == AppLifecycleState.detached) {
        unawaited(leave());
      }
    });
    ref.onDispose(() => unawaited(_teardown(sendLeave: true)));
    return const HuddleState();
  }

  Future<void> start(String parentChannelId) async {
    if (state.isActive) throw StateError('Only one Huddle may be active');
    final generation = ++_generation;
    const uuid = Uuid();
    final channelId = uuid.v4();
    state = HuddleState(
      phase: HuddlePhase.creating,
      parentChannelId: parentChannelId,
      channelId: channelId,
      isCreator: true,
    );
    var created = false;
    var announced = false;
    try {
      final botPubkeys = await _parentBotPubkeys(parentChannelId);
      if (generation != _generation) {
        throw StateError('Huddle start cancelled');
      }
      final relay = _signedRelay();
      await relay.submit(
        kind: 9007,
        content: '',
        tags: buildCreateChannelTags(
          channelId: channelId,
          name: 'Huddle',
          channelType: 'stream',
          visibility: 'private',
          ttlSeconds: 3600,
        ),
      );
      created = true;
      if (generation != _generation) {
        throw StateError('Huddle start cancelled');
      }
      await relay.submit(
        kind: EventKind.huddleGuidelines,
        content: _voiceModeGuidelines(parentChannelId),
        tags: [
          ['h', channelId],
        ],
      );
      if (generation != _generation) {
        throw StateError('Huddle start cancelled');
      }
      if (botPubkeys.isNotEmpty) {
        await ref
            .read(channelActionsProvider)
            .addMembers(
              channelId: channelId,
              pubkeys: botPubkeys.toList(),
              role: 'bot',
            );
      }
      if (generation != _generation) {
        throw StateError('Huddle start cancelled');
      }
      state = state.copyWith(botPubkeys: botPubkeys);
      await relay.submit(
        kind: EventKind.huddleStarted,
        content: jsonEncode({'ephemeral_channel_id': channelId}),
        tags: [
          ['h', parentChannelId],
          ['channel', channelId],
        ],
      );
      announced = true;
      if (generation != _generation) {
        throw StateError('Huddle start cancelled');
      }
      await _connect(generation);
    } catch (error) {
      if (created) {
        try {
          if (announced) {
            await _signedRelay().submit(
              kind: EventKind.huddleEnded,
              content: jsonEncode({'ephemeral_channel_id': channelId}),
              tags: [
                ['h', parentChannelId],
                ['channel', channelId],
              ],
            );
          }
          await _signedRelay().submit(
            kind: 9002,
            content: '',
            tags: buildSetChannelArchivedTags(channelId, archived: true),
          );
        } catch (_) {}
      }
      await _fail(generation, error);
      rethrow;
    }
  }

  Future<Set<String>> _parentBotPubkeys(String parentChannelId) async {
    final loader = _botPubkeysLoader;
    if (loader != null) return loader(parentChannelId);
    final members = await ref.read(
      channelMembersProvider(parentChannelId).future,
    );
    return members
        .where((member) => member.isBot)
        .take(20)
        .map((member) => member.pubkey.toLowerCase())
        .toSet();
  }

  String _voiceModeGuidelines(String parentChannelId) =>
      '''
You are in a live voice huddle attached to channel $parentChannelId.
Your text is read aloud via TTS, message by message, in the order sent.

Latency matters most: send your first sentence immediately, then each following sentence as a separate message.
- If not addressed or relevant: do nothing.
- Keep the whole reply short and start with the answer.
- Use natural speech: no markdown, code blocks, lists, or structured data.
- If a new human message arrives mid-reply, drop unsent sentences and answer the new message.
- Use your Buzz tools proactively when asked.
''';

  Future<void> join({
    required String parentChannelId,
    required String channelId,
  }) async {
    if (state.isActive) throw StateError('Only one Huddle may be active');
    final generation = ++_generation;
    state = HuddleState(
      phase: HuddlePhase.connecting,
      parentChannelId: parentChannelId,
      channelId: channelId,
    );
    try {
      await _connect(generation);
    } catch (error) {
      await _fail(generation, error);
      rethrow;
    }
  }

  SignedEventRelay _signedRelay() {
    final config = ref.read(relayConfigProvider);
    return SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: config.nsec,
    );
  }

  Future<void> _connect(int generation) async {
    if (generation != _generation) return;
    state = state.copyWith(phase: HuddlePhase.connecting, clearError: true);
    final config = ref.read(relayConfigProvider);
    final nsec = config.nsec;
    if (nsec == null || nsec.isEmpty) {
      throw StateError('Huddle requires a signing key');
    }
    final audio = _audioFactory();
    await audio.start();
    if (generation != _generation) {
      await audio.stop();
      return;
    }
    final transport = _transportFactory();
    _audio = audio;
    _transport = transport;
    _transportSub = transport.events.listen(
      (event) => _onTransportEvent(generation, event),
      onError: (Object error, StackTrace stack) =>
          _onDisconnected(generation, error),
    );
    await transport.connect(
      relayWebSocket: Uri.parse(config.wsUrl),
      channelId: state.channelId!,
      parentChannelId: state.parentChannelId,
      nsec: nsec,
    );
    if (generation != _generation) {
      await transport.close(sendLeave: true);
      await audio.stop();
      return;
    }
    final botPubkeys = {
      ...state.botPubkeys,
      ...await _parentBotPubkeys(state.channelId!),
    };
    if (generation != _generation) return;
    state = state.copyWith(botPubkeys: botPubkeys);
    if (botPubkeys.isNotEmpty) {
      _botVoice ??= _botVoiceFactory();
      final subscriber = _messageSubscriber;
      final unsubscribe = subscriber != null
          ? await subscriber(
              state.channelId!,
              (event) => _onBotMessage(generation, event),
            )
          : await ref
                .read(relaySessionProvider.notifier)
                .subscribe(
                  NostrFilter(
                    kinds: const [
                      EventKind.streamMessage,
                      EventKind.streamMessageV2,
                    ],
                    tags: {
                      '#h': [state.channelId!],
                    },
                    since: DateTime.now().millisecondsSinceEpoch ~/ 1000,
                    limit: 200,
                  ),
                  (event) => _onBotMessage(generation, event),
                );
      if (generation != _generation) {
        unsubscribe();
        return;
      }
      _messageUnsubscribe = unsubscribe;
    }
    _microphoneSub = audio.microphonePcm.listen(
      (chunk) {
        _encodeQueue = _encodeQueue
            .then((_) => _onMicrophone(generation, chunk))
            .catchError((Object error) => _onDisconnected(generation, error));
      },
      onError: (Object error, StackTrace stack) =>
          _onDisconnected(generation, error),
    );
    _interruptionSub = audio.interruptions.listen((_) {
      if (generation == _generation) {
        unawaited(leave());
      }
    });
    _reconnectAttempt = 0;
    state = state.copyWith(phase: HuddlePhase.connected, clearError: true);
  }

  Future<void> _onMicrophone(int generation, Int16List chunk) async {
    if (generation != _generation ||
        state.isMuted ||
        state.isTranscribing ||
        state.isBotSpeaking) {
      return;
    }
    for (final pcm in _chunker.add(chunk)) {
      final opus = await _audio!.encode(pcm);
      if (generation != _generation) return;
      final header = _sequence.next(
        levelDbov: audioLevelDbov(pcm),
        isDtx: opus.length <= 2,
      );
      _transport?.sendAudio(Uint8List.fromList([...header.encode(), ...opus]));
    }
  }

  void _onBotMessage(int generation, NostrEvent event) {
    if (generation != _generation ||
        !state.botPubkeys.contains(event.pubkey.toLowerCase()) ||
        !_spokenMessageIds.add(event.id) ||
        event.content.trim().isEmpty) {
      return;
    }
    _speechQueue = _speechQueue
        .then((_) => _speakBotMessage(generation, event.content))
        .catchError((Object error) {
          if (generation == _generation) {
            state = state.copyWith(
              isBotSpeaking: false,
              error: error.toString(),
            );
          }
        });
  }

  Future<void> _speakBotMessage(int generation, String text) async {
    if (generation != _generation || _botVoice == null) return;
    state = state.copyWith(isBotSpeaking: true, clearError: true);
    try {
      await _botVoice!.speak(text);
    } finally {
      if (generation == _generation) {
        state = state.copyWith(isBotSpeaking: false);
      }
    }
  }

  Future<void> talkToBots() async {
    if (state.phase != HuddlePhase.connected || state.botPubkeys.isEmpty) {
      throw const HuddleBotVoiceException(
        'Add a bot to this channel before starting a voice turn.',
      );
    }
    if (state.isTranscribing) {
      await _botVoice?.stopTranscribing();
      return;
    }
    final generation = _generation;
    state = state.copyWith(isTranscribing: true, clearError: true);
    try {
      await _botVoice?.stopSpeaking();
      await _audio?.pauseMicrophone();
      final transcript = await _botVoice!.transcribe();
      if (generation != _generation || transcript == null) return;
      state = state.copyWith(lastTranscript: transcript);
      final sender = _transcriptSender;
      if (sender != null) {
        await sender(state.channelId!, transcript, state.botPubkeys.toList());
      } else {
        await ref.read(sendMessageProvider)(
          channelId: state.channelId!,
          content: transcript,
          mentionPubkeys: state.botPubkeys.toList(),
        );
      }
    } catch (error) {
      if (generation == _generation) {
        state = state.copyWith(error: error.toString());
      }
      rethrow;
    } finally {
      if (generation == _generation) {
        try {
          await _audio?.resumeMicrophone();
        } catch (error) {
          state = state.copyWith(error: error.toString());
        }
        state = state.copyWith(isTranscribing: false);
      }
    }
  }

  void _onTransportEvent(int generation, HuddleTransportEvent event) {
    if (generation != _generation) return;
    switch (event) {
      case HuddleRosterEvent(:final peers):
        state = state.copyWith(
          peers: {for (final peer in peers) peer.peerIndex: peer},
        );
      case HuddlePeerJoinedEvent(:final peer):
        state = state.copyWith(peers: {...state.peers, peer.peerIndex: peer});
      case HuddlePeerLeftEvent(:final peerIndex):
        state = state.copyWith(peers: {...state.peers}..remove(peerIndex));
        _playoutQueues.remove(peerIndex);
        _lastPeerSequence.remove(peerIndex);
        _audio?.removePeer(peerIndex);
      case HuddleAudioFrameEvent(:final peerIndex, :final bytes):
        _decodeQueue = _decodeQueue
            .then((_) => _queueFrame(generation, peerIndex, bytes))
            .catchError((Object error) => _onDisconnected(generation, error));
    }
  }

  Future<void> _queueFrame(
    int generation,
    int peerIndex,
    Uint8List bytes,
  ) async {
    final parsed = HuddleFrameHeader.parse(bytes);
    if (parsed == null || parsed.payload.isEmpty) return;
    final previous = _lastPeerSequence[peerIndex];
    if (previous != null) {
      final delta = (parsed.header.sequence - previous) & 0xffff;
      if (delta == 0 || delta > 0x8000) return;
      if (delta <= 4) {
        for (var missing = 1; missing < delta; missing++) {
          final plc = await _audio?.decode(peerIndex, Uint8List(0), plc: true);
          if (plc != null) _enqueuePlayout(peerIndex, plc);
        }
      }
    }
    _lastPeerSequence[peerIndex] = parsed.header.sequence;
    final pcm = await _audio?.decode(peerIndex, parsed.payload);
    if (pcm == null || generation != _generation) return;
    _enqueuePlayout(peerIndex, pcm);
    _playoutTimer ??= Timer.periodic(
      const Duration(milliseconds: 20),
      (_) => unawaited(_drainPlayout(generation)),
    );
  }

  void _enqueuePlayout(int peerIndex, Int16List pcm) {
    final queue = _playoutQueues.putIfAbsent(peerIndex, ListQueue.new);
    if (queue.length >= 10) queue.removeFirst();
    queue.addLast(pcm);
  }

  Future<void> _drainPlayout(int generation) async {
    if (generation != _generation) return;
    final frames = <Int16List>[];
    for (final queue in _playoutQueues.values) {
      if (queue.isNotEmpty) frames.add(queue.removeFirst());
    }
    _playoutQueues.removeWhere((_, queue) => queue.isEmpty);
    if (frames.isEmpty) {
      _playoutTimer?.cancel();
      _playoutTimer = null;
      return;
    }
    await _audio?.play(mixPcmFrames(frames));
  }

  void _onDisconnected(int generation, Object error) {
    if (generation != _generation || !state.isActive) return;
    if (_reconnectTimer != null) return;
    if (ref.read(appLifecycleProvider) != AppLifecycleState.resumed) {
      unawaited(leave());
      return;
    }
    if (_reconnectAttempt >= 5) {
      unawaited(_fail(generation, error));
      return;
    }
    state = state.copyWith(
      phase: HuddlePhase.reconnecting,
      error: error.toString(),
    );
    final delay = Duration(milliseconds: 250 * (1 << _reconnectAttempt));
    _reconnectAttempt++;
    _reconnectTimer = Timer(delay, () async {
      _reconnectTimer = null;
      await _teardown(sendLeave: false, invalidate: false);
      try {
        await _connect(generation);
      } catch (nextError) {
        _onDisconnected(generation, nextError);
      }
    });
  }

  void toggleMute() => state = state.copyWith(isMuted: !state.isMuted);

  Future<void> setOutputRoute(HuddleOutputRoute route) async {
    try {
      await _audio?.setOutputRoute(route);
      state = state.copyWith(outputRoute: route, clearError: true);
    } catch (error) {
      state = state.copyWith(error: error.toString());
    }
  }

  Future<void> leave() async {
    if (state.isCreator &&
        state.parentChannelId != null &&
        state.channelId != null) {
      try {
        final relay = _signedRelay();
        await relay.submit(
          kind: EventKind.huddleEnded,
          content: jsonEncode({'ephemeral_channel_id': state.channelId}),
          tags: [
            ['h', state.parentChannelId!],
            ['channel', state.channelId!],
          ],
        );
        await relay.submit(
          kind: 9002,
          content: '',
          tags: buildSetChannelArchivedTags(state.channelId!, archived: true),
        );
      } catch (_) {
        // Leaving must always release the microphone and audio session. The
        // ephemeral channel also has a relay-enforced TTL as a final fallback.
      }
    }
    ++_generation;
    await _teardown(sendLeave: true);
    state = const HuddleState();
  }

  Future<void> _fail(int generation, Object error) async {
    if (generation != _generation) return;
    await _teardown(sendLeave: false, invalidate: false);
    state = state.copyWith(phase: HuddlePhase.failed, error: error.toString());
  }

  Future<void> _teardown({
    required bool sendLeave,
    bool invalidate = true,
  }) async {
    if (invalidate) ++_generation;
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    _playoutTimer?.cancel();
    _playoutTimer = null;
    await _microphoneSub?.cancel();
    _microphoneSub = null;
    await _interruptionSub?.cancel();
    _interruptionSub = null;
    _messageUnsubscribe?.call();
    _messageUnsubscribe = null;
    await _transportSub?.cancel();
    _transportSub = null;
    await _transport?.close(sendLeave: sendLeave);
    _transport = null;
    await _audio?.stop();
    _audio = null;
    await _botVoice?.dispose();
    _botVoice = null;
    _chunker.clear();
    _playoutQueues.clear();
    _lastPeerSequence.clear();
    _spokenMessageIds.clear();
  }
}

final huddleControllerProvider =
    NotifierProvider<HuddleController, HuddleState>(HuddleController.new);
