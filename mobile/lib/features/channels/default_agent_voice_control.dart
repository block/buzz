import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/huddle/huddle_session.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/concentric_sheet_surface.dart';
import '../../shared/widgets/agent_navigation_overlay.dart';
import 'voice_note_composer_recorder.dart';
import 'voice_note_recording.dart';
import 'default_agent_voice_draft.dart';
import 'voice_note_waveform.dart';

class _RecordingRouteObserver extends RouteAware {
  _RecordingRouteObserver(this.cancel);
  final VoidCallback cancel;
  @override
  void didPushNext() => cancel();
}

/// Hold-to-record surface anchored to the right edge of the Messages footer.
class DefaultAgentVoiceControl extends HookConsumerWidget {
  const DefaultAgentVoiceControl({
    super.key,
    required this.enabled,
    required this.draftIdentity,
    required this.onRecorded,
    required this.child,
  });

  final bool enabled;
  final String draftIdentity;
  final Future<void> Function(VoiceNoteRecording) onRecorded;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final alive = useRef(true);
    final draft = ref.watch(defaultAgentVoiceDraftProvider(draftIdentity));
    final restoring = useState(true);
    final recorder = useRef<VoiceNoteRecorder?>(null);
    final started = useRef(false);
    final generation = useRef(0);
    final recording = useState(false);
    final sending = useState(false);
    final finalizing = useState(false);
    final restoreFailed = useState(false);
    final pending = useState<VoiceNoteRecording?>(null);
    final samples = useState<List<double>>([]);
    final levels = useRef<StreamSubscription<double>?>(null);
    final timer = useRef<Timer?>(null);
    final error = useState<String?>(null);
    final elapsed = useState(Duration.zero);
    final cancelled = useState(false);

    Future<void> cancel() async {
      generation.value++;
      final current = recorder.value;
      recorder.value = null;
      started.value = false;
      timer.value?.cancel();
      unawaited(levels.value?.cancel());
      if (alive.value && context.mounted) recording.value = false;
      if (current != null) {
        try {
          await current.cancel();
        } finally {
          await current.dispose();
        }
      }
    }

    Future<void> send(VoiceNoteRecording note) async {
      if (sending.value) return;
      sending.value = true;
      pending.value = note;
      error.value = null;
      try {
        final retained = await draft.save(note);
        if (retained.file.path != note.file.path) {
          await deleteDroppedVoiceNoteRecording(note.file.path);
        }
        if (context.mounted) pending.value = retained;
        await onRecorded(retained);
        await draft.clear();
        if (context.mounted) pending.value = null;
      } catch (failure) {
        debugPrint('Default agent voice delivery failed: $failure');
        if (context.mounted) {
          error.value = 'Retry voice note';
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text(
                'Could not confirm delivery. Your voice note is saved.',
              ),
            ),
          );
        }
      } finally {
        if (context.mounted) sending.value = false;
      }
    }

    Future<void> discardPending() async {
      try {
        await draft.clear();
        if (!context.mounted) return;
        pending.value = null;
        restoreFailed.value = false;
        error.value = null;
      } catch (_) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text('Could not discard the saved voice note.'),
            ),
          );
        }
      }
    }

    Future<void> finish() async {
      final current = recorder.value;
      if (current == null) return;
      if (!started.value || cancelled.value) {
        await cancel();
        return;
      }
      finalizing.value = true;
      final finishingGeneration = generation.value;
      recorder.value = null;
      timer.value?.cancel();
      unawaited(levels.value?.cancel());
      if (alive.value && context.mounted) recording.value = false;
      try {
        final note = await current.stop();
        if (!alive.value ||
            !context.mounted ||
            generation.value != finishingGeneration) {
          await deleteDroppedVoiceNoteRecording(note.file.path);
        } else if (note.duration < const Duration(milliseconds: 400)) {
          await deleteDroppedVoiceNoteRecording(note.file.path);
        } else {
          unawaited(HapticFeedback.mediumImpact());
          await send(note);
        }
      } catch (_) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('Could not finish the voice note.')),
          );
        }
      } finally {
        if (alive.value && context.mounted) finalizing.value = false;
        await current.dispose();
      }
    }

    Future<void> start() async {
      if (!enabled ||
          restoring.value ||
          restoreFailed.value ||
          finalizing.value ||
          recorder.value != null ||
          sending.value ||
          pending.value != null) {
        return;
      }
      if (ref.read(huddleSessionProvider).isInSession) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Leave the Huddle before recording a voice note.'),
          ),
        );
        return;
      }
      final current = ref.read(voiceNoteRecorderFactoryProvider)();
      recorder.value = current;
      started.value = false;
      cancelled.value = false;
      unawaited(HapticFeedback.mediumImpact());
      recording.value = true;
      samples.value = [];
      elapsed.value = Duration.zero;
      levels.value = current.levels.listen((level) {
        if (!context.mounted || recorder.value != current) return;
        samples.value = [...samples.value.takeLast(40), level];
      });
      try {
        await current.start();
        if (!context.mounted || recorder.value != current) return;
        started.value = true;
        final watch = Stopwatch()..start();
        timer.value = Timer.periodic(const Duration(milliseconds: 200), (_) {
          if (!context.mounted) return;
          elapsed.value = watch.elapsed;
          if (watch.elapsed >= voiceNoteMaxDuration) unawaited(finish());
        });
      } catch (failure) {
        if (context.mounted && recorder.value == current) {
          await cancel();
          if (context.mounted) {
            ScaffoldMessenger.of(context).showSnackBar(
              SnackBar(
                content: Text(
                  failure is StateError
                      ? failure.message
                      : 'Could not start recording. Check microphone access.',
                ),
              ),
            );
          }
        }
      }
    }

    Future<void> restore() async {
      restoring.value = true;
      try {
        final note = await draft.read();
        if (alive.value && context.mounted) {
          pending.value = note;
          restoreFailed.value = false;
        }
      } catch (_) {
        if (alive.value && context.mounted) {
          restoreFailed.value = true;
          error.value = 'Could not restore voice note. Retry';
        }
      } finally {
        if (alive.value && context.mounted) restoring.value = false;
      }
    }

    useEffect(() {
      unawaited(restore());
      return null;
    }, [draft]);

    final route = ModalRoute.of(context);
    useEffect(() {
      alive.value = true;
      final observer = _RecordingRouteObserver(() => unawaited(cancel()));
      if (route != null) voiceNoteRouteObserver.subscribe(observer, route);
      final lifecycle = AppLifecycleListener(
        onPause: () => unawaited(cancel()),
        onDetach: () => unawaited(cancel()),
      );
      return () {
        alive.value = false;
        voiceNoteRouteObserver.unsubscribe(observer);
        lifecycle.dispose();
        unawaited(cancel());
      };
    }, [route]);

    final active =
        recording.value ||
        finalizing.value ||
        sending.value ||
        pending.value != null ||
        restoreFailed.value;
    final overlay = AgentNavigationOverlay.maybeOf(context);
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (alive.value && context.mounted) overlay?.value = active;
      });
      return null;
    }, [active, overlay]);
    return Semantics(
      value: recording.value
          ? 'Recording ${formatVoiceNoteDuration(elapsed.value)}'
          : null,
      hint: 'Hold to record a voice note. Release to send. Slide up to cancel.',
      customSemanticsActions: enabled && !active
          ? {
              const CustomSemanticsAction(label: 'Record voice note'): () =>
                  unawaited(start()),
            }
          : recording.value
          ? {
              const CustomSemanticsAction(label: 'Discard voice note'): () =>
                  unawaited(cancel()),
            }
          : null,
      child: Listener(
        onPointerCancel: (_) => unawaited(cancel()),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onLongPressStart: enabled ? (_) => unawaited(start()) : null,
          onLongPressMoveUpdate: (details) {
            if (recording.value) {
              final willCancel = details.offsetFromOrigin.dy < -64;
              if (willCancel && !cancelled.value) {
                unawaited(HapticFeedback.selectionClick());
              }
              cancelled.value = willCancel;
            }
          },
          onLongPressEnd: (_) => unawaited(finish()),
          onLongPressCancel: () => unawaited(cancel()),
          child: AnimatedContainer(
            key: const ValueKey('default-agent-voice-pill'),
            duration: MediaQuery.disableAnimationsOf(context)
                ? Duration.zero
                : const Duration(milliseconds: 220),
            curve: Curves.easeInOutCubic,
            width: active ? 224 + Grid.xs * 2 : 56,
            height: 56,
            // Keep one native glass view mounted while the finger is down.
            // Replacing its UIKit subtree cancels the active iOS touch.
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                Positioned.fill(
                  child: ConcentricSheetSurface(
                    enabled: true,
                    usesGlass: true,
                    providesSheetSurface: false,
                    padding: EdgeInsets.zero,
                    minimumRadius: Radii.navigationCapsule,
                    contentClipRadius: Radii.navigationCapsule,
                    child: Stack(
                      fit: StackFit.expand,
                      children: [
                        Positioned.fill(
                          child: !active
                              ? const SizedBox.shrink()
                              : Padding(
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: Grid.xxs,
                                  ),
                                  child: ClipRect(
                                    child: OverflowBox(
                                      alignment: Alignment.centerRight,
                                      minWidth:
                                          224 + Grid.xs * 2 - Grid.xxs * 2,
                                      maxWidth:
                                          224 + Grid.xs * 2 - Grid.xxs * 2,
                                      child: Row(
                                        children: [
                                          if (!recording.value &&
                                              !sending.value &&
                                              (pending.value != null ||
                                                  restoreFailed.value))
                                            IconButton(
                                              tooltip: 'Discard voice note',
                                              onPressed: () =>
                                                  unawaited(discardPending()),
                                              icon: const Icon(Icons.close),
                                            ),
                                          Expanded(
                                            child:
                                                sending.value ||
                                                    finalizing.value
                                                ? const Center(
                                                    child: CircularProgressIndicator(
                                                      semanticsLabel:
                                                          'Sending voice note',
                                                      strokeWidth: 2,
                                                    ),
                                                  )
                                                : restoreFailed.value
                                                ? TextButton(
                                                    onPressed: restoring.value
                                                        ? null
                                                        : () => unawaited(
                                                            restore(),
                                                          ),
                                                    child: Text(
                                                      error.value!,
                                                      maxLines: 2,
                                                    ),
                                                  )
                                                : pending.value != null
                                                ? TextButton(
                                                    onPressed: () => unawaited(
                                                      send(pending.value!),
                                                    ),
                                                    child: Text(
                                                      error.value ??
                                                          'Retry voice note',
                                                    ),
                                                  )
                                                : VoiceNoteWaveform(
                                                    samples: [
                                                      ...List.filled(
                                                        41 -
                                                            samples
                                                                .value
                                                                .length,
                                                        0.0,
                                                      ),
                                                      ...samples.value,
                                                    ],
                                                    progress: 1,
                                                    fadeEdges: true,
                                                    height: 32,
                                                  ),
                                          ),
                                          if (recording.value)
                                            const SizedBox(width: 48),
                                        ],
                                      ),
                                    ),
                                  ),
                                ),
                        ),
                        // The avatar can itself own an iOS video platform view.
                        // Keep it mounted at the same location throughout the hold.
                        Positioned(
                          right: 0,
                          top: 0,
                          width: 56,
                          height: 56,
                          child: ExcludeSemantics(
                            excluding: active,
                            child: IgnorePointer(
                              ignoring: active,
                              child: Opacity(
                                opacity: !active || recording.value ? 1 : 0,
                                child: TooltipTheme(
                                  data: TooltipTheme.of(context).copyWith(
                                    triggerMode: TooltipTriggerMode.manual,
                                  ),
                                  child: child,
                                ),
                              ),
                            ),
                          ),
                        ),
                        if (recording.value)
                          Positioned(
                            right: 0,
                            top: 0,
                            width: 56,
                            height: 56,
                            child: IconButton(
                              tooltip: 'Send voice note',
                              onPressed: () => unawaited(finish()),
                              icon: const SizedBox.square(dimension: 48),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
                if (recording.value)
                  Positioned(
                    bottom: 56 + Grid.xs,
                    width: 224 + Grid.xs * 2,
                    right: 0,
                    child: IgnorePointer(
                      child: Text(
                        cancelled.value
                            ? 'Release to cancel'
                            : 'Slide up to cancel',
                        textAlign: TextAlign.center,
                        style: context.textTheme.labelMedium,
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

extension on List<double> {
  Iterable<double> takeLast(int count) =>
      skip(length > count ? length - count : 0);
}
