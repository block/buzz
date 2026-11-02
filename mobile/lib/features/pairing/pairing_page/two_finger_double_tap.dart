part of '../pairing_page.dart';

class _TwoFingerDoubleTapSurface extends HookWidget {
  const _TwoFingerDoubleTapSurface({required this.child, this.onDoubleTap});

  final Widget child;
  final VoidCallback? onDoubleTap;

  @override
  Widget build(BuildContext context) {
    final taps = useMemoized(_TwoFingerTapTracker.new, [onDoubleTap != null]);
    return Listener(
      behavior: HitTestBehavior.opaque,
      onPointerDown: onDoubleTap == null ? null : taps.down,
      onPointerMove: onDoubleTap == null ? null : taps.move,
      onPointerCancel: onDoubleTap == null ? null : (_) => taps.cancel(),
      onPointerUp: onDoubleTap == null
          ? null
          : (event) => taps.up(event, onDoubleTap!),
      child: child,
    );
  }
}

// Observe two short, stationary two-finger chords without competing with taps
// or scrolling in Flutter's gesture arena.
class _TwoFingerTapTracker {
  final _starts = <int, Offset>{};
  final _down = <int>{};
  Duration _started = Duration.zero;
  Duration? _previousEnd;
  Offset? _previousCenter;
  bool _valid = true;

  void down(PointerDownEvent event) {
    if (_down.isEmpty) {
      _starts.clear();
      _started = event.timeStamp;
      _valid = true;
    }
    _starts[event.pointer] = event.position;
    _down.add(event.pointer);
    if (_starts.length > 2) _valid = false;
  }

  void move(PointerMoveEvent event) {
    final start = _starts[event.pointer];
    if (start != null && (event.position - start).distance > kTouchSlop) {
      _valid = false;
    }
  }

  void cancel() {
    _down.clear();
    _starts.clear();
    _previousEnd = null;
    _previousCenter = null;
    _valid = false;
  }

  void up(PointerUpEvent event, VoidCallback onDoubleTap) {
    if (!_down.remove(event.pointer) || _down.isNotEmpty) return;
    if (!_valid ||
        _starts.length != 2 ||
        event.timeStamp - _started > kDoubleTapTimeout) {
      cancel();
      return;
    }
    final center = (_starts.values.first + _starts.values.last) / 2;
    final gap = _previousEnd == null ? null : _started - _previousEnd!;
    final isDouble =
        gap != null &&
        gap >= kDoubleTapMinTime &&
        gap <= kDoubleTapTimeout &&
        (center - _previousCenter!).distance <= kDoubleTapSlop;
    _previousEnd = event.timeStamp;
    _previousCenter = center;
    if (isDouble) {
      cancel();
      onDoubleTap();
    }
  }
}
