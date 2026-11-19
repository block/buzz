part of '../add_community_page.dart';

/// Observe a developer gesture without delaying Scan's ordinary single tap.
class _PairingRevealSurface extends HookWidget {
  const _PairingRevealSurface({required this.child, this.onReveal});
  final Widget child;
  final VoidCallback? onReveal;

  @override
  Widget build(BuildContext context) {
    final tracker = useMemoized(_PairingRevealTracker.new, [onReveal != null]);
    return Listener(
      behavior: HitTestBehavior.translucent,
      onPointerDown: onReveal == null ? null : tracker.down,
      onPointerMove: onReveal == null ? null : tracker.move,
      onPointerCancel: tracker.pointerCancel,
      onPointerUp: onReveal == null
          ? null
          : (event) => tracker.up(event, onReveal!),
      child: child,
    );
  }
}

class _PairingRevealTracker {
  final _active = <int, PointerDownEvent>{};
  final _positions = <Offset>[];
  Duration? _start;
  Duration? _previousEnd;
  Offset? _previousCenter;
  bool _invalid = false;

  void cancel() {
    _invalid = true;
    _previousEnd = null;
    _previousCenter = null;
  }

  void down(PointerDownEvent event) {
    if (_active.isEmpty) {
      _positions.clear();
      _start = event.timeStamp;
      _invalid = false;
    }
    _active[event.pointer] = event;
    _positions.add(event.position);
    if (_positions.length > 2 ||
        event.timeStamp - _start! > kDoubleTapTimeout) {
      cancel();
    }
  }

  void move(PointerMoveEvent event) {
    final start = _active[event.pointer];
    if (start != null &&
        (event.position - start.position).distance > kTouchSlop) {
      cancel();
    }
  }

  void up(PointerUpEvent event, VoidCallback reveal) {
    final start = _active.remove(event.pointer);
    if (start == null) return;
    if (_positions.length != 2 ||
        (event.position - start.position).distance > kTouchSlop ||
        event.timeStamp - _start! > kDoubleTapTimeout) {
      cancel();
    }
    if (_active.isNotEmpty) return;
    if (_invalid) return;
    final center = (_positions[0] + _positions[1]) / 2;
    final gap = _previousEnd == null ? null : _start! - _previousEnd!;
    if (gap != null &&
        gap >= kDoubleTapMinTime &&
        gap <= kDoubleTapTimeout &&
        (center - _previousCenter!).distance <= kDoubleTapSlop) {
      _previousEnd = null;
      _previousCenter = null;
      reveal();
    } else {
      _previousEnd = event.timeStamp;
      _previousCenter = center;
    }
  }

  void pointerCancel(PointerCancelEvent event) {
    _active.remove(event.pointer);
    cancel();
  }
}
