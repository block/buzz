import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';

/// Desktop's remote-work cue: a cloud that pulses while a remote agent works,
/// and stays still with Reduce Motion.
class RemoteWorkCloud extends HookWidget {
  const RemoteWorkCloud({super.key});

  @override
  Widget build(BuildContext context) {
    // Desktop: 1.4s ease-in-out cycle, dimming to 35% at its midpoint.
    final animation = useAnimationController(
      duration: const Duration(milliseconds: 700),
    );
    // A curve listens to its parent, so build it once and release it.
    final curve = useMemoized(
      () => CurvedAnimation(parent: animation, curve: Curves.easeInOut),
      [animation],
    );
    useEffect(() => curve.dispose, [curve]);
    final opacity = useMemoized(
      () => Tween<double>(begin: 1, end: 0.35).animate(curve),
      [curve],
    );
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    useEffect(() {
      if (reducedMotion) {
        animation.value = 0;
      } else {
        animation.repeat(reverse: true);
      }
      return animation.stop;
    }, [animation, reducedMotion]);
    return FadeTransition(
      key: const ValueKey('remote-work-cloud'),
      opacity: opacity,
      child: Icon(
        BuzzIcons.cloud,
        size: 16,
        color: context.colors.onSurfaceVariant,
      ),
    );
  }
}
