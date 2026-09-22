import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/app_lifecycle_provider.dart';
import '../../shared/theme/theme.dart';
import 'voice_note_recording.dart';
import 'voice_note_waveform.dart';

class _VoiceNoteRouteAware extends RouteAware {
  _VoiceNoteRouteAware(this.onCovered);

  final VoidCallback onCovered;

  @override
  void didPushNext() => onCovered();
}

/// Composer control that records, previews levels, and finalizes a voice note.
class VoiceNoteComposerRecorder extends HookConsumerWidget {
  const VoiceNoteComposerRecorder({
    super.key,
    required this.onCancel,
    required this.onRecorded,
  });

  final VoidCallback onCancel;
  final ValueChanged<VoiceNoteRecording> onRecorded;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final recorder = useMemoized(ref.read(voiceNoteRecorderFactoryProvider));
    final samples = useState<List<double>>(const []);
    final sampleSequence = useState(0);
    final elapsed = useState(Duration.zero);
    final error = useState<String?>(null);
    final isStarted = useState(false);
    final isStopping = useState(false);
    final startedAt = useRef<DateTime?>(null);
    final routeAware = useMemoized(
      () => _VoiceNoteRouteAware(() {
        if (context.mounted) onCancel();
      }),
      [onCancel],
    );

    useEffect(() {
      final subscription = ref.listenManual(appLifecycleProvider, (
        previous,
        next,
      ) {
        if (next != AppLifecycleState.paused &&
            next != AppLifecycleState.detached) {
          return;
        }
        unawaited(recorder.cancel());
        if (context.mounted) onCancel();
      });
      return subscription.close;
    }, [recorder, onCancel]);

    final route = ModalRoute.of(context);
    useEffect(() {
      if (route != null) voiceNoteRouteObserver.subscribe(routeAware, route);
      return () => voiceNoteRouteObserver.unsubscribe(routeAware);
    }, [routeAware, route]);

    Future<void> finish() async {
      if (!isStarted.value || isStopping.value || error.value != null) return;
      isStopping.value = true;
      unawaited(HapticFeedback.mediumImpact());
      try {
        final recording = await recorder.stop();
        if (context.mounted) {
          onRecorded(recording);
        } else {
          await deleteDroppedVoiceNoteRecording(recording.file.path);
        }
      } catch (_) {
        if (context.mounted) {
          error.value = 'Buzz could not finish the voice note.';
          isStopping.value = false;
        }
      }
    }

    useEffect(() {
      var active = true;
      final levelSubscription = recorder.levels.listen((level) {
        if (!active) return;
        final nextSamples = [...samples.value, level];
        samples.value = nextSamples.length <= 120
            ? nextSamples
            : nextSamples.sublist(nextSamples.length - 120);
        sampleSequence.value += 1;
      });
      final timer = Timer.periodic(const Duration(milliseconds: 200), (_) {
        final started = startedAt.value;
        if (!active || started == null) return;
        elapsed.value = DateTime.now().difference(started);
        if (elapsed.value >= voiceNoteMaxDuration) unawaited(finish());
      });
      unawaited(() async {
        try {
          await recorder.start();
          if (active) {
            startedAt.value = DateTime.now();
            isStarted.value = true;
          }
        } on StateError catch (recordingError) {
          if (active) error.value = recordingError.message;
        } catch (_) {
          if (active) {
            error.value =
                'Buzz could not start recording. Check microphone access.';
          }
        }
      }());
      return () {
        active = false;
        timer.cancel();
        unawaited(levelSubscription.cancel());
        unawaited(() async {
          await recorder.cancel();
          await recorder.dispose();
        }());
      };
    }, [recorder]);

    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    return Row(
      key: const ValueKey('voice-note-recorder'),
      children: [
        _RecorderButton(
          key: const ValueKey('voice-note-recorder-close'),
          tooltip: 'Discard voice note',
          icon: LucideIcons.x,
          foreground: context.colors.onSurfaceVariant,
          background: context.colors.surface,
          onPressed: isStopping.value ? null : onCancel,
        ),
        const SizedBox(width: Grid.half),
        if (error.value case final message?)
          Expanded(
            child: Text(
              message,
              key: const ValueKey('voice-note-recorder-error'),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.error,
              ),
            ),
          )
        else ...[
          Text(
            '${formatVoiceNoteDuration(elapsed.value)} / '
            '${formatVoiceNoteDuration(voiceNoteMaxDuration)}',
            key: const ValueKey('voice-note-recorder-duration'),
            style: context.textTheme.labelSmall?.copyWith(
              color: context.colors.onSurfaceVariant,
              fontFeatures: const [FontFeature.tabularFigures()],
            ),
          ),
          const SizedBox(width: Grid.half),
          Expanded(
            child: LayoutBuilder(
              builder: (context, constraints) {
                final barCount = ((constraints.maxWidth + 2) / 5).floor().clamp(
                  1,
                  1024,
                );
                final recentSamples = samples.value.length <= barCount
                    ? samples.value
                    : samples.value.sublist(samples.value.length - barCount);
                final waveform = [
                  ...List<double>.filled(barCount - recentSamples.length, 0),
                  ...recentSamples,
                ];
                return ClipRect(
                  child: TweenAnimationBuilder<double>(
                    key: ValueKey(sampleSequence.value),
                    tween: Tween(begin: reducedMotion ? 0 : 5, end: 0),
                    duration: reducedMotion
                        ? Duration.zero
                        : const Duration(milliseconds: 90),
                    curve: Curves.linear,
                    builder: (context, offset, child) => Transform.translate(
                      offset: Offset(offset, 0),
                      child: child,
                    ),
                    child: VoiceNoteWaveform(
                      samples: waveform,
                      progress: 1,
                      fadeEdges: true,
                      height: 24,
                      minimumBarHeight: 3,
                      maximumBarHeight: 20,
                      colorOpacity: 0.75,
                    ),
                  ),
                );
              },
            ),
          ),
        ],
        const SizedBox(width: Grid.half),
        _RecorderButton(
          key: const ValueKey('voice-note-recorder-stop'),
          tooltip: 'Stop recording',
          icon: LucideIcons.square,
          foreground: Colors.white,
          background: context.colors.error,
          onPressed: error.value == null && isStarted.value && !isStopping.value
              ? finish
              : null,
        ),
      ],
    );
  }
}

/// Best-effort deletion for a finalized recording the composer cannot retain.
Future<void> deleteDroppedVoiceNoteRecording(String path) async {
  try {
    final file = File(path);
    if (await file.exists()) await file.delete();
  } catch (_) {
    // Best-effort cleanup must not escape an already unmounted recorder.
  }
}

class _RecorderButton extends StatelessWidget {
  const _RecorderButton({
    super.key,
    required this.tooltip,
    required this.icon,
    required this.foreground,
    required this.background,
    required this.onPressed,
  });

  final String tooltip;
  final IconData icon;
  final Color foreground;
  final Color background;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) => SizedBox.square(
    dimension: 36,
    child: IconButton(
      tooltip: tooltip,
      onPressed: onPressed == null
          ? null
          : () {
              unawaited(HapticFeedback.selectionClick());
              onPressed!();
            },
      style: IconButton.styleFrom(
        foregroundColor: foreground,
        backgroundColor: background,
        disabledBackgroundColor: background.withValues(alpha: 0.5),
        shape: const CircleBorder(),
        side: BorderSide(color: Colors.black.withValues(alpha: 0.04), width: 1),
      ),
      padding: EdgeInsets.zero,
      visualDensity: VisualDensity.compact,
      icon: Icon(icon, size: 18),
    ),
  );
}
