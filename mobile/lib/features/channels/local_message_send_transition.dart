import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'local_message_send_animation_provider.dart';
import 'local_send_row.dart';

/// Duration of a local message's entrance transition.
const localMessageSendTransitionDuration = Duration(milliseconds: 500);

// A damped spring contracts quickly, gently passes the target width, then
// settles. Normalize the endpoint so there is no snap when the clock completes.
final _contractionSpring = _SendContractionSpring();

class _SendContractionSpring extends Curve {
  final _simulation = SpringSimulation(
    SpringDescription.withDampingRatio(mass: 1, stiffness: 256, ratio: 0.82),
    0,
    1,
    4,
  );

  double get _seconds =>
      localMessageSendTransitionDuration.inMicroseconds /
      Duration.microsecondsPerSecond;

  @override
  double transformInternal(double t) =>
      _simulation.x(t * _seconds) / _simulation.x(_seconds);
}

/// Default travel multiplier for a message entrance.
const localMessageSendTransitionStartOffset = 1.5;

/// Travel multiplier used when the message includes a sender header.
const localMessageSendTransitionAvatarStartOffset = 2.5;

/// Coordinates upward row expansion and the bubble flight from the composer.
class LocalMessageSendTransition extends HookConsumerWidget {
  /// Creates a local send transition around [child].
  const LocalMessageSendTransition({
    super.key,
    required this.animate,
    this.channelId,
    this.messageId,
    this.threadHeadId,
    this.startOffsetFactor = localMessageSendTransitionStartOffset,
    required this.child,
  });

  /// Composer scope used to retrieve the captured sending surface.
  final String? channelId;

  /// Event whose local claim owns the submitted composer geometry.
  final String? messageId;
  final String? threadHeadId;

  /// Whether to play the entrance transition.
  final bool animate;

  /// Initial vertical offset expressed as a multiple of 24 logical pixels.
  final double startOffsetFactor;

  /// Message row whose bottom stays anchored during expansion.
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final origin = useMemoized(
      () => !animate || channelId == null || messageId == null
          ? null
          : ref
                .read(localMessageSendAnimationProvider(channelId!).notifier)
                .originFor(messageId!),
      const [],
    );
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(
      duration: localMessageSendTransitionDuration,
      initialValue: animate && !reduceMotion ? 0 : 1,
    );

    useEffect(() {
      if (animate && !reduceMotion) {
        unawaited(
          controller.animateTo(
            1,
            duration: localMessageSendTransitionDuration,
            curve: Curves.linear,
          ),
        );
      } else {
        controller.value = 1;
      }
      return null;
    }, [animate, controller, reduceMotion]);

    return AnimatedBuilder(
      animation: controller,
      child: child,
      builder: (context, child) => LocalSendProgress(
        progress: const Interval(
          0.15,
          1,
          curve: Curves.easeInOutCubic,
        ).transform(controller.value),
        widthProgress: _contractionSpring.transform(controller.value),
        origin: origin,
        startOffset: 24 * startOffsetFactor,
        // History follows the lift, while width contracts ahead of it.
        child: LocalSendRow(
          progress: const Interval(
            0.15,
            1,
            curve: Curves.easeInOutCubic,
          ).transform(controller.value),
          child: child!,
        ),
      ),
    );
  }
}

/// Shares the local entrance progress with bubble backgrounds beneath the row.
class LocalSendProgress extends InheritedWidget {
  /// Creates a progress scope for the message row.
  const LocalSendProgress({
    super.key,
    required this.progress,
    this.widthProgress,
    this.origin,
    this.startOffset = 36,
    required super.child,
  });

  /// Horizontal contraction, independent of vertical travel.
  /// Defaults to [progress] for surfaces without a staged entrance.
  final double? widthProgress;

  /// Captured composer rectangle in global coordinates.
  final Rect? origin;

  /// Fallback travel when no composer geometry is available.
  final double startOffset;

  /// Entrance progress from zero (full width) to one (settled).
  final double progress;

  /// Geometry and progress inherited by the bubble's paint surface.
  static LocalSendProgress? scopeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<LocalSendProgress>();

  /// Returns settled progress for content outside a local-send transition.
  static double of(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<LocalSendProgress>()
          ?.progress ??
      1;

  @override
  bool updateShouldNotify(LocalSendProgress oldWidget) =>
      progress != oldWidget.progress ||
      widthProgress != oldWidget.widthProgress;
}
