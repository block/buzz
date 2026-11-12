import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Tracks the height of a viewport after Flutter has laid it out.
///
/// Create one instance for a viewport, pass it to [LaidOutViewportReporter],
/// listen to [height] for measurements, and call [dispose] when the owner is
/// removed. Measurements are reported after the frame that performed layout.
class LaidOutViewport {
  /// Key used by [LaidOutViewportReporter] to locate the viewport render box.
  final key = GlobalKey();

  /// The latest laid-out viewport height, or zero before the first report.
  final height = ValueNotifier(0.0);

  /// Schedules a viewport measurement after the current frame is laid out.
  ///
  /// Reports only changes of at least half a logical pixel. If the viewport is
  /// not mounted or has not been laid out, the scheduled report does nothing.
  void reportAfterLayout() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final renderObject = key.currentContext?.findRenderObject();
      if (renderObject is! RenderBox || !renderObject.hasSize) return;
      final nextHeight = renderObject.size.height;
      if ((height.value - nextHeight).abs() < 0.5) return;
      height.value = nextHeight;
    });
  }

  /// Releases the [height] notifier owned by this viewport.
  void dispose() => height.dispose();
}

/// Reports the laid-out height of [child] through [viewport].
///
/// Reports once after initial layout and again whenever the child's layout size
/// changes. The owner must keep [viewport] stable across rebuilds and dispose it
/// after this reporter is removed.
class LaidOutViewportReporter extends HookWidget {
  /// Receives the laid-out height measurements.
  final LaidOutViewport viewport;

  /// The widget whose viewport height is measured.
  final Widget child;

  /// Creates a reporter for [child] backed by [viewport].
  const LaidOutViewportReporter({
    super.key,
    required this.viewport,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    useEffect(() {
      viewport.reportAfterLayout();
      return null;
    }, [viewport]);

    return NotificationListener<SizeChangedLayoutNotification>(
      onNotification: (_) {
        viewport.reportAfterLayout();
        return true;
      },
      child: SizeChangedLayoutNotifier(
        child: KeyedSubtree(key: viewport.key, child: child),
      ),
    );
  }
}
