import 'dart:async';
import 'package:buzz/features/channels/bestie_transcription_loading_indicator.dart';
import 'dart:io';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../shared/me/me_sync_test.dart' show FakeRelay;
import 'package:buzz/features/channels/bestie_transcription_control.dart';
import 'package:buzz/features/channels/bestie_transcription_draft.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/features/channels/voice_note_waveform.dart';
import 'package:buzz/features/channels/bestie_transcription_status.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:buzz/shared/transcription/device_transcription.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';

class _Store extends BestieTranscriptStore {
  _Store() : super('test', () async => Directory.systemTemp);
  BestieTranscriptDraft? saved;
  Completer<void>? retention;
  @override
  Future<BestieTranscriptDraft?> read() async => saved;
  @override
  Future<BestieTranscriptDraft> retain(
    Future<VoiceNoteRecording> recording,
    String recipient,
  ) async {
    await recording;
    await retention?.future;
    return saved = BestieTranscriptDraft(
      id: 'one',
      recipient: recipient,
      audioPath: '/saved.wav',
    );
  }

  @override
  Future<BestieTranscriptDraft> update(
    BestieTranscriptDraft draft, {
    String? text,
    NostrEvent? event,
  }) async {
    return saved = BestieTranscriptDraft(
      id: draft.id,
      recipient: draft.recipient,
      audioPath: draft.audioPath,
      text: text ?? saved?.text,
      event: event ?? saved?.event,
    );
  }

  @override
  Future<void> clear({String? id}) async {
    saved = null;
  }
}

class _Recorder implements VoiceNoteRecorder {
  bool cancelled = false;
  int stops = 0;
  final amplitudes = StreamController<double>.broadcast();
  @override
  Stream<double> get levels => amplitudes.stream;
  @override
  Future<void> start() async {}
  @override
  Future<VoiceNoteRecording> stop() async {
    stops++;
    return VoiceNoteRecording(
      file: XFile('/local.wav'),
      duration: const Duration(seconds: 3),
      waveform: [0.5],
    );
  }

  @override
  Future<void> cancel() async {
    cancelled = true;
  }

  @override
  Future<void> dispose() async {}
}

class _Speech implements DeviceTranscription {
  final result = Completer<String>();
  Completer<void>? preparation;
  int transcriptions = 0;
  bool cancelled = false;
  @override
  Future<void> prepare() async {
    await preparation?.future;
  }

  @override
  Future<String> transcribe(String path) {
    transcriptions++;
    return result.future;
  }

  @override
  Future<void> cancel() async {
    cancelled = true;
  }
}

void main() {
  late _Store store;
  late _Recorder recorder;
  late _Speech speech;
  late FakeRelay relay;
  late MeSync sync;
  late ValueNotifier<bool> showControl;
  late ConversationPaneController pane;
  void reset() {
    store = _Store();
    showControl = ValueNotifier(true);
    pane = ConversationPaneController();
    recorder = _Recorder();
    speech = _Speech();
    relay = FakeRelay();
    sync = MeSync(
      relay,
      RelayConfig(
        baseUrl: 'https://transcript.test',
        nsec: nostr.Keys.generate().nsec,
      ),
    );
  }

  Future<void> mount(WidgetTester tester) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          meSyncProvider.overrideWithValue(sync),
          relaySessionProvider.overrideWith(() => relay),
          channelsProvider.overrideWith(_NoChannels.new),
          channelActionsProvider.overrideWithValue(_Actions()),
          sendMessageProvider.overrideWithValue(
            SendMessage(
              signedEventRelay: SignedEventRelay(
                session: relay,
                nsec: sync.config.nsec,
              ),
              fetchMembers: (_) async => [
                ChannelMember(
                  pubkey: sync.pubkey,
                  role: 'member',
                  joinedAt: DateTime(2026),
                ),
                ChannelMember(
                  pubkey: 'a' * 64,
                  role: 'bot',
                  joinedAt: DateTime(2026),
                ),
              ],
              readUserCache: () => {},
              addLocalMessage: (_, _) {},
              completeLocalMessage: (_, _) {},
              removeLocalMessage: (_, _) {},
            ),
          ),
          bestieTranscriptStoreProvider(sync.cacheKey).overrideWithValue(store),
          bestieTranscriptionRecorderProvider.overrideWithValue(() => recorder),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Center(
              child: ConversationPaneScope(
                controller: pane,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const BestieTranscriptionStatus(
                      channelId: '11111111-1111-4111-8111-111111111111',
                    ),
                    ValueListenableBuilder<bool>(
                      valueListenable: showControl,
                      builder: (_, visible, _) => !visible
                          ? const SizedBox()
                          : BestieTranscriptionControl(
                              scope: sync.cacheKey,
                              recipient: 'a' * 64,
                              child: SizedBox(
                                width: 56,
                                height: 56,
                                child: IconButton(
                                  tooltip: 'Chat with your Bestie',
                                  onPressed: () {},
                                  icon: const Text('Bestie'),
                                ),
                              ),
                            ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<TestGesture> hold(WidgetTester tester) async {
    final gesture = await tester.startGesture(
      tester.getCenter(find.text('Bestie')),
    );
    await tester.pump(const Duration(milliseconds: 600));
    if (speech.preparation == null) {
      await tester.pumpAndSettle();
      expect(find.byType(VoiceNoteWaveform), findsOneWidget);
      final pill = tester.widget<AnimatedContainer>(
        find.byKey(const ValueKey('default-agent-voice-pill')),
      );
      expect(pill.duration, const Duration(milliseconds: 220));
      expect(pill.curve, Curves.easeInOutCubic);
      expect(pill.constraints?.maxWidth, 224 + Grid.xs * 2);
    } else {
      await tester.pump();
    }
    return gesture;
  }

  testWidgets(
    'release sends finalized text with no media and retries the identical signed event',
    (tester) async {
      reset();
      relay.loseAcknowledgment = true;
      await mount(tester);
      final gesture = await hold(tester);
      await gesture.up();
      await tester.pump();
      speech.result.complete('Please summarize my open tasks.');
      await tester.pumpAndSettle();
      expect(
        relay.events,
        hasLength(1),
        reason: tester
            .widgetList<Text>(find.byType(Text))
            .map((t) => t.data)
            .join(' | '),
      );
      final sent = relay.events.single;
      expect(sent.content, 'Please summarize my open tasks.');
      expect(sent.tags.where((tag) => tag.first == 'imeta'), isEmpty);
      expect(sent.tags, contains(equals(['p', 'a' * 64])));
      expect(store.saved?.event?.id, sent.id);
      relay.loseAcknowledgment = false;
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      expect(relay.writes, 2);
      expect(relay.events.single.id, sent.id);
      expect(speech.transcriptions, 1);
      expect(store.saved, isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'release retains original recipient and transcribes the saved audio',
    (tester) async {
      reset();
      await mount(tester);
      final gesture = await hold(tester);
      expect(find.byType(VoiceNoteWaveform), findsOneWidget);
      recorder.amplitudes.add(.8);
      await tester.pump();
      store.retention = Completer<void>();
      await gesture.up();
      await tester.pump();
      // Finish the pill collapse while the processing spinner keeps animating.
      await tester.pump(const Duration(milliseconds: 220));
      expect(find.byTooltip('Discard transcript'), findsNothing);
      expect(find.byType(BestieTranscriptionLoadingIndicator), findsOneWidget);
      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(
        tester
            .widget<AnimatedContainer>(
              find.byKey(const ValueKey('default-agent-voice-pill')),
            )
            .constraints
            ?.maxWidth,
        56,
      );
      store.retention!.complete();
      await tester.pump();
      expect(recorder.stops, 1);
      expect(store.saved?.recipient, 'a' * 64);
      expect(speech.transcriptions, 1);
      expect(pane.selectedId, '11111111-1111-4111-8111-111111111111');
      await tester.pump();
      expect(find.text('Transcribing…'), findsNothing);
      expect(find.byTooltip('Discard transcript'), findsNothing);
      expect(find.byType(BestieTranscriptionLoadingIndicator), findsOneWidget);
      expect(
        tester
            .widget<AnimatedContainer>(
              find.byKey(const ValueKey('default-agent-voice-pill')),
            )
            .constraints
            ?.maxWidth,
        56,
      );
      speech.result.complete('');
      await tester.pumpAndSettle();
      expect(store.saved, isNotNull);
      expect(find.text('Retry'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'slide up cancels without transcribing or retaining a recording',
    (tester) async {
      reset();
      await mount(tester);
      final gesture = await hold(tester);
      await gesture.moveBy(const Offset(0, -90));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(recorder.cancelled, isTrue);
      expect(recorder.stops, 0);
      expect(speech.transcriptions, 0);
      expect(store.saved, isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'release during model preparation cannot later open the microphone',
    (tester) async {
      reset();
      speech.preparation = Completer<void>();
      await mount(tester);
      final gesture = await hold(tester);
      await gesture.up();
      await tester.pump();
      speech.preparation!.complete();
      await tester.pumpAndSettle();
      expect(speech.cancelled, isTrue);
      expect(recorder.stops, 0);
      expect(find.byType(VoiceNoteWaveform), findsNothing);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets('disposing the account scope prevents late transcript delivery', (
    tester,
  ) async {
    reset();
    await mount(tester);
    final gesture = await hold(tester);
    await gesture.up();
    await tester.pump();
    expect(speech.transcriptions, 1);
    await tester.pumpWidget(const SizedBox());
    speech.result.complete('Must remain saved');
    await tester.pumpAndSettle();
    expect(relay.events, isEmpty);
    expect(store.saved, isNotNull);
    expect(speech.cancelled, isTrue);
    expect(tester.takeException(), isNull);
  });
  testWidgets(
    'leaving Home for chat keeps transcription running and sends there',
    (tester) async {
      reset();
      await mount(tester);
      final gesture = await hold(tester);
      await gesture.up();
      await tester.pump();
      expect(speech.transcriptions, 1);
      expect(pane.selectedId, '11111111-1111-4111-8111-111111111111');
      await tester.pump();
      expect(find.text('Transcribing…'), findsNothing);
      expect(find.byTooltip('Discard transcript'), findsNothing);
      expect(find.byType(BestieTranscriptionLoadingIndicator), findsOneWidget);
      expect(
        tester
            .widget<AnimatedContainer>(
              find.byKey(const ValueKey('default-agent-voice-pill')),
            )
            .constraints
            ?.maxWidth,
        56,
      );
      showControl.value = false;
      await tester.pump();
      speech.result.complete('Send this in the opened chat');
      await tester.pumpAndSettle();
      expect(relay.events.single.content, 'Send this in the opened chat');
      expect(store.saved, isNull);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
}

class _NoChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

class _Actions implements ChannelActions {
  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async => Channel(
    id: '11111111-1111-4111-8111-111111111111',
    name: 'Bestie',
    channelType: 'dm',
    visibility: 'private',
    description: '',
    createdBy: 'a' * 64,
    createdAt: DateTime(2026),
    memberCount: 2,
    participantPubkeys: pubkeys,
    isMember: true,
  );
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
