import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// A single vertical modal body whose leading-edge drags move its existing
/// bottom-sheet route. Only this explicit scroll position participates; nested
/// scrollables and horizontal gestures retain their ordinary behavior.
///
/// The route owns its animation and chrome. Outside an enabled, fully entered
/// modal bottom sheet this is an ordinary, always-scrollable body.
class ModalScrollHandoff extends HookConsumerWidget {
  /// Creates an opt-in body for a modal bottom sheet.
  const ModalScrollHandoff({super.key, required this.child});

  /// The scrollable body, excluding the sheet's handle and header.
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useMemoized(() => _HandoffController(() => context));
    useEffect(() => controller.dispose, [controller]);
    return SingleChildScrollView(
      controller: controller,
      primary: false,
      physics: const AlwaysScrollableScrollPhysics(),
      child: child,
    );
  }
}

// Flutter 3.47.9, material/bottom_sheet.dart: _kMinFlingVelocity and
// _kCloseProgressThreshold. Keep the body policy aligned with the stock handle,
// except that a deliberate upward release vetoes the distance-based close and
// that velocity alone closes only a gesture that never scrolled content.
const _minFlingVelocity = 700.0;
const _closeProgressThreshold = 0.5;

class _SheetBinding {
  _SheetBinding(this.element, this.route, this.sheet, this.reduceMotion);

  final Element element;
  final ModalBottomSheetRoute<dynamic> route;
  final BottomSheet sheet;
  final bool reduceMotion;
  AnimationController get animation => sheet.animationController!;

  bool get recoverable =>
      element.mounted &&
      route.isActive &&
      animation.status != AnimationStatus.reverse;

  bool get current => recoverable && route.isCurrent;

  double? get height {
    if (!element.mounted) return null;
    final box = element.findRenderObject();
    if (box is! RenderBox || !box.hasSize) return null;
    final height = box.size.height;
    return height.isFinite && height > 0 ? height : null;
  }

  static _SheetBinding? read(BuildContext context) {
    if (!context.mounted) return null;
    final route = ModalRoute.of(context);
    if (route is! ModalBottomSheetRoute || !route.isCurrent) return null;
    _SheetBinding? binding;
    context.visitAncestorElements((element) {
      final widget = element.widget;
      if (widget is! BottomSheet) return true;
      if (widget.enableDrag && widget.animationController != null) {
        binding = _SheetBinding(
          element,
          route,
          widget,
          MediaQuery.disableAnimationsOf(context),
        );
      }
      return false;
    });
    return binding;
  }
}

class _HandoffController extends ScrollController {
  _HandoffController(this.bodyContext);
  final BuildContext Function() bodyContext;

  @override
  ScrollPosition createScrollPosition(
    ScrollPhysics physics,
    ScrollContext context,
    ScrollPosition? oldPosition,
  ) => _HandoffPosition(
    owner: this,
    physics: physics,
    context: context,
    oldPosition: oldPosition,
  );

  @override
  void detach(ScrollPosition position) {
    if (position is _HandoffPosition) position.interrupt();
    super.detach(position);
  }
}

class _HandoffPosition extends ScrollPositionWithSingleContext {
  _HandoffPosition({
    required this.owner,
    required super.physics,
    required super.context,
    super.oldPosition,
  });

  final _HandoffController owner;
  Object? _gesture;
  _SheetBinding? _sheet;
  DragStartDetails? _start;
  // Whether content has been away from its leading edge at any point in the
  // current gesture. A flick that scrolled content back to the top must not
  // dismiss on velocity alone.
  bool _scrolledContent = false;
  bool _suppressBallistic = false;

  // A terminal token is cleared before delegating end/cancel: Flutter disposes
  // its ScrollDragController during normal activity replacement as well.
  void interrupt() => _finish(null);

  @override
  void absorb(ScrollPosition other) {
    if (other is _HandoffPosition) other.interrupt();
    super.absorb(other);
  }

  @override
  void beginActivity(ScrollActivity? newActivity) {
    if (newActivity != null) interrupt();
    super.beginActivity(newActivity);
  }

  @override
  Drag drag(DragStartDetails details, VoidCallback dragCancelCallback) {
    interrupt();
    final drag = super.drag(details, dragCancelCallback);
    // A displaced release that never went ballistic belongs to the old gesture.
    _suppressBallistic = false;
    _scrolledContent = false;
    final token = Object();
    _gesture = token;
    _start = details;
    return _HandoffDrag(drag, (end) {
      if (_gesture != token) return;
      _finish(end);
    });
  }

  // Within the physics tolerance content counts as resting at the top, so a
  // finger-placed fraction of a pixel does not disable the velocity close.
  bool get _awayFromLeadingEdge =>
      pixels - minScrollExtent > physics.toleranceFor(this).distance;

  @override
  void applyUserOffset(double delta) {
    // Sampled before every update, so it sees content the gesture began on
    // and content an earlier update scrolled, before this one returns it.
    _scrolledContent |= _awayFromLeadingEdge;
    if (_gesture == null ||
        axisDirection != AxisDirection.down ||
        owner.positions.length != 1) {
      super.applyUserOffset(delta);
      return;
    }
    final binding = _sheet ?? _SheetBinding.read(owner.bodyContext());
    final height = binding?.height;
    if (binding == null ||
        !binding.current ||
        height == null ||
        (_sheet == null && !binding.animation.isCompleted)) {
      interrupt();
      super.applyUserOffset(delta);
      return;
    }
    updateUserScrollDirection(
      delta > 0 ? ScrollDirection.forward : ScrollDirection.reverse,
    );
    // Repay inherited top bounce before handing any remainder to the route.
    // Downward motion while already overscrolled stays with platform physics.
    if (pixels < minScrollExtent) {
      if (delta >= 0) {
        super.applyUserOffset(delta);
        return;
      }
      final repay = math.min(-delta, minScrollExtent - pixels);
      setPixels(pixels + repay);
      delta += repay;
    }
    if (delta > 0) {
      final content = math.min(delta, math.max(0.0, pixels - minScrollExtent));
      setPixels(pixels - content);
      delta -= content;
    } else if (delta < 0 && _sheet != null) {
      final reopen = math.min(-delta, (1 - binding.animation.value) * height);
      _moveSheet(binding, reopen / height);
      delta += reopen;
    }
    if (delta > 0) {
      _moveSheet(binding, -delta / height);
    } else if (delta < 0) {
      super.applyUserOffset(delta);
    }
  }

  void _moveSheet(_SheetBinding binding, double progress) {
    if (progress == 0) return;
    if (_sheet == null) {
      _sheet = binding;
      binding.sheet.onDragStart?.call(_start!);
    }
    binding.animation.value = (binding.animation.value + progress).clamp(0, 1);
  }

  void _finish(DragEndDetails? end) {
    _gesture = null;
    _start = null;
    final binding = _sheet;
    _sheet = null;
    if (binding == null || !binding.recoverable) return;
    final animation = binding.animation;
    final displaced = animation.value < 1;
    final velocity = end?.velocity.pixelsPerSecond.dy ?? 0.0;
    final closing =
        end != null &&
        binding.route.isCurrent &&
        displaced &&
        ((!_scrolledContent && velocity > _minFlingVelocity) ||
            (velocity >= -_minFlingVelocity &&
                animation.value < _closeProgressThreshold));
    binding.sheet.onDragEnd?.call(end ?? DragEndDetails(), isClosing: closing);
    _suppressBallistic = displaced;
    if (binding.reduceMotion || animation.duration == Duration.zero) {
      animation.value = closing ? 0 : 1;
    } else if (closing) {
      final height = binding.height;
      animation.fling(
        velocity: velocity > _minFlingVelocity && height != null
            ? -velocity / height
            : -1,
      );
    } else {
      animation.forward();
    }
    if (closing) binding.sheet.onClosing();
  }

  @override
  void goBallistic(double velocity) {
    // Synthetic ballistic transitions are interruptions, never releases.
    interrupt();
    final suppress = _suppressBallistic;
    _suppressBallistic = false;
    super.goBallistic(suppress ? 0 : velocity);
  }

  @override
  void dispose() {
    // Never animate a departing route from final disposal.
    _gesture = null;
    _sheet = null;
    super.dispose();
  }
}

class _HandoffDrag implements Drag {
  _HandoffDrag(this.delegate, this.finish);
  final Drag delegate;
  final void Function(DragEndDetails?) finish;

  @override
  void update(DragUpdateDetails details) => delegate.update(details);

  @override
  void end(DragEndDetails details) {
    finish(details);
    delegate.end(details);
  }

  @override
  void cancel() {
    finish(null);
    delegate.cancel();
  }
}
