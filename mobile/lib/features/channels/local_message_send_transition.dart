import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Duration of a local message's entrance transition.
const localMessageSendTransitionDuration = Duration(milliseconds: 280);

/// Default travel multiplier for a message entrance.
const localMessageSendTransitionStartOffset = 1.5;

/// Travel multiplier used when the message includes a sender header.
const localMessageSendTransitionAvatarStartOffset = 2.5;

/// Slides a newly sent local message upward from beneath the composer.
class LocalMessageSendTransition extends HookWidget {
  /// Creates a local send transition around [child].
  const LocalMessageSendTransition({
    super.key,
    required this.animate,
    this.startOffsetFactor = localMessageSendTransitionStartOffset,
    required this.child,
  });

  /// Whether to play the entrance transition.
  final bool animate;

  /// Initial vertical offset expressed as a multiple of 24 logical pixels.
  final double startOffsetFactor;

  /// Message content translated by the transition.
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(
      duration: localMessageSendTransitionDuration,
      initialValue: animate && !reduceMotion ? 0 : 1,
    );
    final easedAnimation = useMemoized(
      () => CurvedAnimation(parent: controller, curve: Curves.easeOutCubic),
      [controller],
    );
    useEffect(() => easedAnimation.dispose, [easedAnimation]);

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
      animation: easedAnimation,
      child: child,
      builder: (context, child) => Transform.translate(
        offset: Offset(0, 24 * startOffsetFactor * (1 - easedAnimation.value)),
        transformHitTests: false,
        child: Transform.scale(
          scale: 0.96 + 0.04 * easedAnimation.value,
          alignment: Alignment.bottomLeft,
          transformHitTests: false,
          child: child,
        ),
      ),
    );
  }
}
