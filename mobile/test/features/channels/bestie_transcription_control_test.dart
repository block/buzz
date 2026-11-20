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
import 'package:buzz/features/channels/bestie_transcription_job.dart';
import 'package:buzz/features/channels/bestie_transcription_surface.dart';
import 'package:buzz/shared/widgets/app_route_observer.dart';
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
  Object? retentionFailure;
  Completer<void>? deletion;
  @override
  Future<BestieTranscriptDraft?> read() async => saved;
  @override
  Future<BestieTranscriptDraft> retain(
    Future<VoiceNoteRecording> recording,
    String recipient,
  ) async {
    await recording;
    await retention?.future;
    saved = BestieTranscriptDraft(
      id: 'one',
      recipient: recipient,
      audioPath: '/saved.wav',
    );
    if (retentionFailure case final failure?) throw failure;
    return saved!;
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
    await deletion?.future;
    saved = null;
  }
}

class _Recorder implements VoiceNoteRecorder {
  bool cancelled = false;
  int stops = 0;
  int starts = 0;
  final amplitudes = StreamController<double>.broadcast();
  @override
  Stream<double> get levels => amplitudes.stream;
  @override
  Future<void> start() async {
    starts++;
  }

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
  Object? preparationFailure;
  int transcriptions = 0;
  bool cancelled = false;
  @override
  Future<bool> isAvailable() async => true;
  @override
  Future<void> prepare() async {
    await preparation?.future;
    if (preparationFailure case final failure?) throw failure;
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
  late _Actions actions;
  late _Recorder recorder;
  late _Speech speech;
  late FakeRelay relay;
  late MeSync sync;
  late ValueNotifier<bool> showControl;
  late ConversationPaneController pane;
  void reset() {
    store = _Store();
    actions = _Actions();
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
          channelActionsProvider.overrideWithValue(actions),
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
          navigatorObservers: [appRouteObserver],
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
      expect(pane.selectedId, isNull);
      await tester.tap(find.text('Retry transcript'));
      await tester.pumpAndSettle();
      expect(relay.writes, 2);
      expect(relay.events.single.id, sent.id);
      expect(speech.transcriptions, 1);
      expect(store.saved, isNull);
      expect(pane.selectedId, '11111111-1111-4111-8111-111111111111');
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'discard reserves the job before deletion and rejects concurrent retry',
    (tester) async {
      reset();
      relay.loseAcknowledgment = true;
      await mount(tester);
      final gesture = await hold(tester);
      await gesture.up();
      await tester.pump();
      speech.result.complete('Keep this signed event');
      await tester.pumpAndSettle();
      expect(relay.writes, 1);
      final container = ProviderScope.containerOf(
        tester.element(find.byType(BestieTranscriptionStatus)),
      );
      final job = container.read(bestieTranscriptionJobProvider(sync.cacheKey));
      store.deletion = Completer<void>();
      final discard = job.discard();
      await tester.pump();
      expect(job.busy, isTrue);
      await job.retry();
      expect(relay.writes, 1);
      expect(store.saved, isNotNull);
      store.deletion!.complete();
      await discard;
      await tester.pumpAndSettle();
      expect(store.saved, isNull);
      expect(job.busy, isFalse);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Home recovery cannot retry while discard is deleting its journal',
    (tester) async {
      reset();
      store.saved = const BestieTranscriptDraft(
        id: 'one',
        recipient: 'agent',
        audioPath: '/saved.wav',
      );
      await mount(tester);
      final surface = tester.widget<BestieTranscriptionSurface>(
        find.byType(BestieTranscriptionSurface),
      );
      store.deletion = Completer<void>();
      final discard = surface.discardPending();
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await surface.retry();
      expect(speech.transcriptions, 0);
      expect(relay.events, isEmpty);
      store.deletion!.complete();
      await discard;
      await tester.pumpAndSettle();
      expect(store.saved, isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('retry reserves Home recovery while the destination opens', (
    tester,
  ) async {
    reset();
    store.saved = BestieTranscriptDraft(
      id: 'one',
      recipient: 'a' * 64,
      audioPath: '/saved.wav',
      text: 'Retained text',
    );
    await mount(tester);
    final surface = tester.widget<BestieTranscriptionSurface>(
      find.byType(BestieTranscriptionSurface),
    );
    actions.opening = Completer<void>();
    final retry = surface.retry();
    await tester.pump();
    await surface.discardPending();
    expect(store.saved, isNotNull);
    expect(relay.events, isEmpty);
    actions.opening!.complete();
    await retry;
    await tester.pumpAndSettle();
    expect(relay.events.single.content, 'Retained text');
    expect(store.saved, isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets(
    'unsupported file recognition fails before opening the microphone',
    (tester) async {
      reset();
      speech.preparationFailure = StateError(
        'Transcribing saved recordings is not available on Android yet.',
      );
      await mount(tester);
      final gesture = await tester.startGesture(
        tester.getCenter(find.text('Bestie')),
      );
      await tester.pump(const Duration(milliseconds: 600));
      await gesture.up();
      await tester.pumpAndSettle();
      expect(recorder.starts, 0);
      expect(speech.transcriptions, 0);
      expect(relay.events, isEmpty);
      expect(find.textContaining('not available on Android'), findsWidgets);
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
      expect(pane.selectedId, isNull);
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
  Future<void> expectRetainedAfterInterruption(
    WidgetTester tester, {
    required Future<void> Function() interrupt,
    required Future<void> Function() resume,
    bool failAfterJournal = false,
  }) async {
    reset();
    await mount(tester);
    final gesture = await hold(tester);
    store.retention = Completer<void>();
    await gesture.up();
    await tester.pump();
    expect(recorder.stops, 1);
    await interrupt();
    // Let cancellation's stream teardown settle outside the fake clock.
    await tester.runAsync(() => Future<void>(() {}));
    await resume();
    await tester.pump();
    // The slow save still owns the slot, so a second hold cannot start.
    final second = await tester.startGesture(
      tester.getCenter(find.byType(BestieTranscriptionSurface)),
    );
    await tester.pump(const Duration(milliseconds: 600));
    await second.up();
    await tester.pump();
    expect(recorder.stops, 1);
    expect(
      tester
          .widget<BestieTranscriptionSurface>(
            find.byType(BestieTranscriptionSurface),
          )
          .finalizing,
      isTrue,
    );
    if (failAfterJournal) store.retentionFailure = StateError('disk');
    store.retention!.complete();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    final surface = tester.widget<BestieTranscriptionSurface>(
      find.byType(BestieTranscriptionSurface),
    );
    expect(store.saved, isNotNull);
    expect(speech.transcriptions, 0);
    expect(surface.hasPending, isTrue);
    expect(find.text('Retry transcript'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    if (!speech.result.isCompleted) speech.result.complete('');
    await tester.pumpAndSettle();
  }

  testWidgets(
    'pausing during retention still exposes the saved utterance on resume',
    (tester) async {
      await expectRetainedAfterInterruption(
        tester,
        interrupt: () async {
          tester.binding.handleAppLifecycleStateChanged(
            AppLifecycleState.paused,
          );
          await tester.pump();
        },
        resume: () async => tester.binding.handleAppLifecycleStateChanged(
          AppLifecycleState.resumed,
        ),
      );
    },
  );
  testWidgets(
    'a save that fails after journaling during a pause still offers Retry',
    (tester) async {
      await expectRetainedAfterInterruption(
        tester,
        failAfterJournal: true,
        interrupt: () async {
          tester.binding.handleAppLifecycleStateChanged(
            AppLifecycleState.paused,
          );
          await tester.pump();
        },
        resume: () async => tester.binding.handleAppLifecycleStateChanged(
          AppLifecycleState.resumed,
        ),
      );
    },
  );
  testWidgets(
    'pushing a route during retention still exposes the saved utterance on return',
    (tester) async {
      await expectRetainedAfterInterruption(
        tester,
        interrupt: () async {
          unawaited(
            tester
                .state<NavigatorState>(find.byType(Navigator))
                .push(
                  MaterialPageRoute<void>(builder: (_) => const SizedBox()),
                ),
          );
          await tester.pump();
        },
        resume: () async =>
            tester.state<NavigatorState>(find.byType(Navigator)).pop(),
      );
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
      expect(find.text('Preparing speech…'), findsOneWidget);
      expect(find.byTooltip('Send transcript'), findsNothing);
      expect(find.byType(VoiceNoteWaveform), findsNothing);
      await gesture.up();
      await tester.pump();
      expect(recorder.starts, 0);
      speech.preparation!.complete();
      await tester.pumpAndSettle();
      expect(speech.cancelled, isTrue);
      expect(find.textContaining('Speech was not ready'), findsWidgets);
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
    'leaving Home during processing sends without reopening the conversation',
    (tester) async {
      reset();
      await mount(tester);
      final gesture = await hold(tester);
      await gesture.up();
      await tester.pump();
      showControl.value = false;
      await tester.pump();
      speech.result.complete('Finish in the background');
      await tester.pumpAndSettle();
      expect(relay.events.single.content, 'Finish in the background');
      expect(store.saved, isNull);
      expect(pane.selectedId, isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Home keeps the Bestie loader until successful delivery then opens chat',
    (tester) async {
      reset();
      await mount(tester);
      final gesture = await hold(tester);
      await gesture.up();
      await tester.pump();
      expect(speech.transcriptions, 1);
      expect(pane.selectedId, isNull);
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
      speech.result.complete('Send this in the opened chat');
      await tester.pumpAndSettle();
      expect(relay.events.single.content, 'Send this in the opened chat');
      expect(pane.selectedId, '11111111-1111-4111-8111-111111111111');
      expect(find.byType(BestieTranscriptionLoadingIndicator), findsNothing);
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
  Completer<void>? opening;
  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    await opening?.future;
    return Channel(
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
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
