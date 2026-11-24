import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:record/record.dart';

import '../../shared/huddle/huddle_session.dart';
import '../../shared/transcription/device_transcription.dart';
import '../../shared/widgets/app_route_observer.dart';
import 'bestie_transcription_job.dart';
import 'bestie_transcription_draft.dart';
import 'voice_note_recording.dart';
import 'bestie_transcription_surface.dart';
import 'bestie_conversation.dart';
import '../../shared/recording_haptics.dart';

/// Independent PCM capture; ordinary composer voice notes keep their AAC format.
final bestieTranscriptionRecorderProvider =
    Provider<VoiceNoteRecorder Function()>(
      (ref) =>
          () => DeviceVoiceNoteRecorder(
            recordConfig: const RecordConfig(
              encoder: AudioEncoder.wav,
              iosConfig: IosRecordConfig(
                allowHapticsAndSystemSoundsDuringRecording: true,
              ),
              sampleRate: 16000,
              numChannels: 1,
              autoGain: true,
              echoCancel: true,
              noiseSuppress: true,
            ),
            fileExtension: 'wav',
          ),
    );

class _DictationRouteObserver extends RouteAware {
  _DictationRouteObserver(this.cancel);
  final VoidCallback cancel;
  @override
  void didPushNext() => cancel();
}

/// Hold the Bestie button to capture, release to transcribe and send plain text.
class BestieTranscriptionControl extends HookConsumerWidget {
  const BestieTranscriptionControl({
    super.key,
    required this.scope,
    required this.recipient,
    required this.child,
  });
  final String scope;
  final String? recipient;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final store = ref.watch(bestieTranscriptStoreProvider(scope));
    final job = ref.watch(bestieTranscriptionJobProvider(scope));
    final progress = useValueListenable(job);
    final phase = useState('Restoring');
    final error = useState<String?>(null);
    final draft = useState<BestieTranscriptDraft?>(null);
    final recorder = useRef<VoiceNoteRecorder?>(null);
    final speech = useRef<DeviceTranscription?>(null);
    final generation = useRef(0);
    final started = useRef(false);
    final cancelOnRelease = useState(false);
    final levels = useRef<StreamSubscription<double>?>(null);
    final samples = useState<List<double>>([]);
    final timer = useRef<Timer?>(null);
    final elapsed = useState(Duration.zero);
    final alive = useRef(true);
    bool current(int token) =>
        alive.value && context.mounted && token == generation.value;
    void report(Object failure) {
      if (!alive.value || !context.mounted) return;
      error.value = failure is PlatformException
          ? failure.message ?? 'Transcription failed.'
          : failure is StateError
          ? failure.message
          : 'Could not finish transcription. Retry or discard the saved recording.';
      phase.value = '';
    }

    Future<void> cancel() async {
      // Release has already committed this send. A route/lifecycle callback
      // cannot revoke a publication that may have reached the relay.
      if (phase.value == 'Sending') return;
      final cancellation = ++generation.value;
      timer.value?.cancel();
      final capture = recorder.value;
      recorder.value = null;
      started.value = false;
      final transcription = speech.value;
      speech.value = null;
      // Stop microphone and recognition immediately; meter teardown must not
      // hold either resource open while its stream subscription settles.
      final stopCapture = capture == null
          ? Future<void>.value()
          : capture.cancel().whenComplete(capture.dispose);
      final stopSpeech = transcription?.cancel().catchError((Object _) {});
      await Future.wait<void>([
        stopCapture,
        ?stopSpeech,
        if (levels.value != null) levels.value!.cancel(),
      ]);
      // A released recording still being saved keeps Home busy until its
      // retention settles, so no new hold can race it for the slot.
      if (current(cancellation) &&
          phase.value != 'Sending' &&
          phase.value != 'Saving' &&
          phase.value != 'Discarding') {
        phase.value = '';
      }
    }

    Future<void> restore() async {
      try {
        final saved = await store.read();
        if (!alive.value || !context.mounted) return;
        draft.value = job.value.draft == null ? saved : null;
        phase.value = '';
      } catch (failure) {
        report(failure);
      }
    }

    Future<void> process(BestieTranscriptDraft pending, int token) async {
      phase.value = 'Opening conversation';
      try {
        final channel = await job.destination(pending);
        if (!current(token)) return;
        // Keep progress on Home, but let delivery survive the user leaving it.
        final delivery = job.run(pending, channel);
        draft.value = null;
        phase.value = '';
        final sent = await delivery;
        if (!sent || !current(token) || !context.mounted) return;
        unawaited(openBestieDestination(context, channel));
      } catch (failure) {
        if (current(token)) report(failure);
      }
    }

    Future<void> finish() async {
      if (phase.value != 'Listening') {
        if (phase.value == 'Preparing speech') {
          await cancel();
          report(
            StateError(
              'Speech was not ready. Hold again and wait for Listening before speaking.',
            ),
          );
        }
        return;
      }
      if (cancelOnRelease.value || !started.value) {
        await cancel();
        return;
      }
      final capture = recorder.value;
      if (capture == null || phase.value != 'Listening') return;
      final destination = recipient;
      if (destination == null) {
        await cancel();
        return;
      }
      recorder.value = null;
      started.value = false;
      timer.value?.cancel();
      unawaited(levels.value?.cancel());
      final token = generation.value;
      phase.value = 'Saving';
      try {
        final saved = await store.retain(capture.stop(), destination);
        if (!alive.value || !context.mounted) return;
        // The slot is occupied even if a route push or backgrounding cancelled
        // this release, so keep it visible for Retry and block new captures.
        draft.value = saved;
        if (!current(token)) {
          phase.value = '';
          return;
        }
        unawaited(HapticFeedback.mediumImpact());
        await process(saved, token);
      } catch (failure) {
        // Reconcile with the slot even after a cancel: a save that failed
        // after journaling still left a draft to retry or discard.
        if (alive.value && context.mounted) {
          await restore();
          if (current(token)) report(failure);
        }
      } finally {
        await capture.dispose();
      }
    }

    Future<void> start() async {
      if (recipient == null ||
          phase.value.isNotEmpty ||
          draft.value != null ||
          job.busy ||
          progress.draft != null ||
          error.value != null) {
        return;
      }
      if (ref.read(huddleSessionProvider).isInSession) {
        report(StateError('Leave the Huddle before dictating.'));
        return;
      }
      final token = ++generation.value;
      phase.value = 'Preparing speech';
      elapsed.value = Duration.zero;
      cancelOnRelease.value = false;
      samples.value = [];
      final native = ref.read(deviceTranscriptionProvider)();
      speech.value = native;
      try {
        await native.prepare().timeout(const Duration(minutes: 2));
        if (!current(token)) return;
        final capture = ref.read(bestieTranscriptionRecorderProvider)();
        recorder.value = capture;
        levels.value = capture.levels.listen((value) {
          if (current(token)) {
            samples.value = [
              ...samples.value.skip(samples.value.length > 40 ? 1 : 0),
              value,
            ];
          }
        });
        await capture.start();
        if (!current(token)) return;
        started.value = true;
        phase.value = 'Listening';
        unawaited(recordingHaptic());
        final watch = Stopwatch()..start();
        timer.value = Timer.periodic(const Duration(milliseconds: 200), (_) {
          if (!current(token)) return;
          elapsed.value = watch.elapsed;
          if (watch.elapsed >= voiceNoteMaxDuration) unawaited(finish());
        });
      } catch (failure) {
        if (current(token)) {
          await cancel();
          report(failure);
        }
      }
    }

    useEffect(() {
      unawaited(restore());
      return () {
        alive.value = false;
        unawaited(cancel());
      };
    }, const []);
    useEffect(() {
      if (recorder.value != null || speech.value != null) unawaited(cancel());
      return null;
    }, [recipient]);
    useOnAppLifecycleStateChange((_, next) {
      if (next == AppLifecycleState.paused ||
          next == AppLifecycleState.detached) {
        unawaited(cancel());
      }
    });
    final route = ModalRoute.of(context);
    useEffect(() {
      if (route == null) return null;
      final observer = _DictationRouteObserver(() => unawaited(cancel()));
      appRouteObserver.subscribe(observer, route);
      return () => appRouteObserver.unsubscribe(observer);
    }, [route]);
    return BestieTranscriptionSurface(
      enabled: recipient != null && !job.busy && progress.draft == null,
      recording: phase.value == 'Listening',
      preparing: phase.value == 'Preparing speech',
      processingLabel: job.busy
          ? '${progress.phase} transcript'
          : phase.value == 'Discarding'
          ? 'Discarding transcript'
          : 'Saving recording',
      finalizing:
          phase.value == 'Saving' || phase.value == 'Opening conversation',
      sending: job.busy || phase.value == 'Discarding',
      restoring: phase.value == 'Restoring',
      restoreFailed:
          error.value != null && draft.value == null && progress.draft == null,
      hasPending: draft.value != null || (!job.busy && progress.draft != null),
      elapsed: elapsed.value,
      samples: samples.value,
      error:
          error.value ??
          (progress.error == null
              ? null
              : 'Could not send transcript. Recording saved.'),
      cancelled: cancelOnRelease,
      start: start,
      finish: finish,
      cancel: cancel,
      retry: () async {
        final pending = draft.value ?? progress.draft;
        if (pending == null || phase.value.isNotEmpty) return;
        error.value = null;
        await process(pending, ++generation.value);
      },
      restore: () async {
        error.value = null;
        await restore();
      },
      discardPending: () async {
        if (phase.value.isNotEmpty) return;
        phase.value = 'Discarding';
        try {
          if (job.value.draft != null) {
            await job.discard();
          } else {
            await store.clear();
          }
          if (context.mounted) {
            draft.value = null;
            error.value = null;
            phase.value = '';
          }
        } catch (failure) {
          report(failure);
        }
      },
      child: child,
    );
  }
}
