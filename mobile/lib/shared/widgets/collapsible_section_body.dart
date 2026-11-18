import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Duration of a section opening and its chevron rotation.
const Duration sectionExpandDuration = Duration(milliseconds: 220);

/// Duration of a section closing.
const Duration sectionCollapseDuration = Duration(milliseconds: 170);

/// Easing shared by section bodies and chevrons.
const Curve sectionExpandCurve = Cubic(0.23, 1, 0.32, 1);

/// Easing for the closing section body.
const Curve sectionCollapseCurve = Curves.easeInCubic;

/// Subtle vertical compression at the collapsed end of the transition.
const double sectionCollapsedScaleY = 0.98;

/// Shared Channels and Me section expansion, including reduced-motion support.
class CollapsibleSectionBody extends HookWidget {
  final bool expanded;
  final Widget child;

  const CollapsibleSectionBody({
    super.key,
    required this.expanded,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final controller = useAnimationController(
      duration: reducedMotion ? Duration.zero : sectionExpandDuration,
      reverseDuration: reducedMotion ? Duration.zero : sectionCollapseDuration,
      initialValue: expanded ? 1 : 0,
    );
    final curvedAnimation = useMemoized(
      () => CurvedAnimation(
        parent: controller,
        curve: sectionExpandCurve,
        reverseCurve: sectionCollapseCurve,
      ),
      [controller],
    );
    final shouldRender = useState(expanded);

    useEffect(() => curvedAnimation.dispose, [curvedAnimation]);

    useEffect(() {
      void handleStatus(AnimationStatus status) {
        if (status == AnimationStatus.dismissed && !expanded) {
          shouldRender.value = false;
        }
      }

      controller.addStatusListener(handleStatus);
      return () => controller.removeStatusListener(handleStatus);
    }, [controller, expanded]);

    useEffect(() {
      controller.duration = reducedMotion
          ? Duration.zero
          : sectionExpandDuration;
      controller.reverseDuration = reducedMotion
          ? Duration.zero
          : sectionCollapseDuration;

      if (expanded) {
        shouldRender.value = true;
        if (reducedMotion) {
          controller.value = 1;
        } else {
          unawaited(controller.forward());
        }
      } else if (reducedMotion) {
        controller.value = 0;
        shouldRender.value = false;
      } else {
        unawaited(controller.reverse());
      }

      return null;
    }, [controller, expanded, reducedMotion]);

    return ClipRect(
      child: AnimatedBuilder(
        animation: curvedAnimation,
        child: shouldRender.value ? child : const SizedBox.shrink(),
        builder: (context, child) {
          final value = curvedAnimation.value.clamp(0.0, 1.0);
          final scaleY =
              sectionCollapsedScaleY + ((1 - sectionCollapsedScaleY) * value);

          return Align(
            alignment: Alignment.topCenter,
            heightFactor: value,
            child: Opacity(
              opacity: value,
              child: Transform.scale(
                alignment: Alignment.topCenter,
                scaleY: scaleY,
                child: ExcludeSemantics(
                  excluding: !expanded,
                  child: IgnorePointer(ignoring: !expanded, child: child),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}
