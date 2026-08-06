import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// An [InkWell] whose long press is observed above interactive descendants.
class MessageLongPressInkWell extends StatelessWidget {
  final VoidCallback? onTap;
  final ValueChanged<Rect> onLongPress;
  final BorderRadius? borderRadius;
  final Color? highlightColor;
  final Widget child;

  const MessageLongPressInkWell({
    super.key,
    this.onTap,
    required this.onLongPress,
    this.borderRadius,
    this.highlightColor,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    return _MessageLongPressRegion(
      onLongPress: onLongPress,
      child: InkWell(
        onTap: onTap,
        borderRadius: borderRadius,
        highlightColor: highlightColor,
        child: child,
      ),
    );
  }
}

/// Detects a message long press without competing with interactive descendants.
///
/// Links, media, reactions, and other nested controls keep their normal tap
/// gestures. Moving far enough to scroll cancels the timer; recognizing the
/// hold cancels the pointer so a descendant tap cannot fire on release.
class _MessageLongPressRegion extends HookWidget {
  final ValueChanged<Rect> onLongPress;
  final Widget child;

  const _MessageLongPressRegion({
    required this.onLongPress,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final activePointer = useRef<int?>(null);
    final origin = useRef<Offset?>(null);
    final timer = useRef<Timer?>(null);

    void cancel() {
      timer.value?.cancel();
      timer.value = null;
      activePointer.value = null;
      origin.value = null;
    }

    useEffect(() => cancel, const []);

    void recognize() {
      final renderObject = context.findRenderObject();
      if (renderObject is! RenderBox || !renderObject.hasSize) return;
      onLongPress(renderObject.localToGlobal(Offset.zero) & renderObject.size);
    }

    void handlePointerDown(PointerDownEvent event) {
      if (activePointer.value != null) return;
      activePointer.value = event.pointer;
      origin.value = event.position;
      timer.value = Timer(kLongPressTimeout, () {
        final pointer = activePointer.value;
        if (pointer == null) return;
        timer.value = null;
        activePointer.value = null;
        origin.value = null;
        GestureBinding.instance.cancelPointer(pointer);
        recognize();
      });
    }

    void handlePointerMove(PointerMoveEvent event) {
      if (event.pointer != activePointer.value) return;
      final start = origin.value;
      if (start == null) return;
      final delta = event.position - start;
      if (delta.distanceSquared > kTouchSlop * kTouchSlop) cancel();
    }

    void handlePointerEnd(PointerEvent event) {
      if (event.pointer == activePointer.value) cancel();
    }

    return Semantics(
      onLongPress: recognize,
      child: Listener(
        behavior: HitTestBehavior.translucent,
        onPointerDown: handlePointerDown,
        onPointerMove: handlePointerMove,
        onPointerUp: handlePointerEnd,
        onPointerCancel: handlePointerEnd,
        child: child,
      ),
    );
  }
}
