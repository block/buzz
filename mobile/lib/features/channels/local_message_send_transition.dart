import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

const localMessageSendTransitionDuration = Duration(milliseconds: 300);
const localMessageSendTransitionStartOffset = 1.5;
const localMessageSendTransitionAvatarStartOffset = 2.5;

/// Slides a newly sent local message upward from beneath the composer.
class LocalMessageSendTransition extends HookWidget {
  const LocalMessageSendTransition({
    super.key,
    required this.animate,
    this.startOffsetFactor = localMessageSendTransitionStartOffset,
    required this.child,
  });

  final bool animate;
  final double startOffsetFactor;
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
      builder: (context, child) => FractionalTranslation(
        translation: Offset(0, startOffsetFactor * (1 - easedAnimation.value)),
        transformHitTests: false,
        child: child,
      ),
    );
  }
}
