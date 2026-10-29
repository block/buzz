import 'package:buzz/features/channels/voice_note_composer_recorder.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:buzz/shared/widgets/app_route_observer.dart';
import 'package:flutter/material.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:buzz/shared/widgets/modal_presentation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  for (final rootSheet in [false, true]) {
    testWidgets('covering route cancels recording (root sheet: $rootSheet)', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(1000, 700);
      addTearDown(tester.view.reset);
      final pane = ConversationPaneController();
      final otherPane = ConversationPaneController();
      final recording = ValueNotifier(true);
      addTearDown(pane.dispose);
      addTearDown(otherPane.dispose);
      addTearDown(recording.dispose);
      final recorder = _Recorder();
      var cancellations = 0;
      var reveals = 0;
      void onReveal() => reveals++;
      late BuildContext detailContext;
      pane.open('channel', (context) {
        detailContext = context;
        return Scaffold(
          body: ValueListenableBuilder<bool>(
            valueListenable: recording,
            builder: (_, active, _) => active
                ? VoiceNoteComposerRecorder(
                    onCancel: () {
                      cancellations++;
                      recording.value = false;
                    },
                    onRecorded: (_) =>
                        fail('Covered recording must be discarded'),
                  )
                : const Text('Composer restored'),
          ),
        );
      });
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            voiceNoteRecorderFactoryProvider.overrideWithValue(() => recorder),
          ],
          child: MaterialApp(
            navigatorObservers: [appRouteObserver],
            home: Stack(
              children: [
                AdaptiveConversationPane(
                  controller: pane,
                  strip: const FoldStrip(
                    side: FoldStripSide.right,
                    rect: Rect.fromLTWH(924, 0, 76, 700),
                    topClearance: 110,
                    bottomClearance: 0,
                    isClearanceEstimated: true,
                  ),
                  list: const Text('Channels'),
                ),
                Offstage(
                  child: AdaptiveConversationPane(
                    controller: otherPane,
                    list: const Text('Other tab'),
                  ),
                ),
              ],
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(recorder.started, isTrue);
      expect(recorder.cancelled, isFalse);
      final route = ModalRoute.of(detailContext)!;
      appRouteObserver.addRevealListener(route, onReveal);
      addTearDown(() => appRouteObserver.removeRevealListener(route, onReveal));
      final nested = Navigator.of(detailContext);
      expect(nested, isNot(Navigator.of(detailContext, rootNavigator: true)));

      // Folding retains the recorder; only a covering route should cancel it.
      tester.view.physicalSize = const Size(400, 700);
      await tester.pumpAndSettle();
      tester.view.physicalSize = const Size(1000, 700);
      await tester.pumpAndSettle();
      expect(cancellations, 0);
      expect(recorder.cancelled, isFalse);

      if (rootSheet) {
        showBuzzModalBottomSheet<void>(
          context: detailContext,
          builder: (_) => const Text('Channel details'),
        );
      } else {
        nested.push(
          MaterialPageRoute<void>(
            builder: (_) => const Scaffold(body: Text('Channel details')),
          ),
        );
      }
      await tester.pumpAndSettle();
      expect(find.text('Channel details'), findsOneWidget);
      expect(cancellations, 1);
      expect(recorder.cancelled, isTrue);
      expect(recorder.disposed, isTrue);
      expect(
        find.byType(VoiceNoteComposerRecorder, skipOffstage: false),
        findsNothing,
      );

      Navigator.of(detailContext, rootNavigator: rootSheet).pop();
      await tester.pumpAndSettle();
      expect(find.text('Composer restored'), findsOneWidget);
      expect(reveals, rootSheet ? 0 : 1);
      expect(cancellations, 1);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    });
  }
}

class _Recorder implements VoiceNoteRecorder {
  bool started = false;
  bool cancelled = false;
  bool disposed = false;

  @override
  Stream<double> get levels => const Stream.empty();

  @override
  Future<void> start() async => started = true;

  @override
  Future<void> cancel() async => cancelled = true;

  @override
  Future<void> dispose() async => disposed = true;

  @override
  Future<VoiceNoteRecording> stop() async =>
      throw StateError('Unexpected save');
}
