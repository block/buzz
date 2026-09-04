import 'package:flutter/material.dart';

import 'jump_to_latest_button.dart';

/// Whether channel Latest should remain hidden at the effective timeline tail.
///
/// Composer and keyboard layout updates can briefly make item-position
/// measurements stale. Preserve an active tail-follow intent through those
/// frames unless the user explicitly detached from the tail.
bool shouldHideChannelJumpToLatest({
  required bool isAtLatest,
  required bool followsLatest,
  required bool userHasDetached,
}) => isAtLatest || (followsLatest && !userHasDetached);

/// Shared channel/thread visibility transition for [JumpToLatestButton].
class JumpToLatestSwitcher extends StatelessWidget {
  final String id;
  final bool visible;
  final VoidCallback onPressed;

  const JumpToLatestSwitcher({
    required this.id,
    required this.visible,
    required this.onPressed,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final child = visible
        ? JumpToLatestButton(
            key: ValueKey('$id-jump-to-latest'),
            id: id,
            onPressed: onPressed,
          )
        : SizedBox.shrink(key: ValueKey('$id-jump-to-latest-hidden'));
    if (Theme.of(context).platform == TargetPlatform.iOS) {
      return KeyedSubtree(
        key: ValueKey('$id-jump-to-latest-switcher'),
        child: child,
      );
    }
    return AnimatedSwitcher(
      key: ValueKey('$id-jump-to-latest-switcher'),
      duration: reduceMotion
          ? Duration.zero
          : const Duration(milliseconds: 180),
      reverseDuration: reduceMotion
          ? Duration.zero
          : const Duration(milliseconds: 160),
      switchInCurve: Curves.easeOutCubic,
      switchOutCurve: Curves.easeInCubic,
      transitionBuilder: (child, animation) => FadeTransition(
        opacity: animation,
        child: ScaleTransition(
          scale: _JumpToLatestScaleAnimation(animation),
          alignment: Alignment.bottomCenter,
          child: child,
        ),
      ),
      child: child,
    );
  }
}

class _JumpToLatestScaleAnimation extends Animation<double>
    with AnimationWithParentMixin<double> {
  @override
  final Animation<double> parent;

  _JumpToLatestScaleAnimation(this.parent);

  @override
  double get value => parent.status == AnimationStatus.reverse
      ? parent.value
      : 0.92 + (0.08 * parent.value);
}
