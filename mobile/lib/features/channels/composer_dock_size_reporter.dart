import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Reports the laid-out height of a floating composer dock.
///
/// Message timelines use that height as scroll padding while still painting
/// beneath the dock, which lets the dock's fade reveal real timeline content.
class ComposerDockSizeReporter extends HookWidget {
  final ValueChanged<double> onHeightChanged;
  final Widget child;

  const ComposerDockSizeReporter({
    super.key,
    required this.onHeightChanged,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final sizeKey = useMemoized(GlobalKey.new);
    final lastHeight = useRef<double?>(null);

    void reportHeight() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        final renderObject = sizeKey.currentContext?.findRenderObject();
        if (renderObject is! RenderBox || !renderObject.hasSize) return;
        final height = renderObject.size.height;
        final previous = lastHeight.value;
        if (previous != null && (previous - height).abs() < 0.5) return;
        lastHeight.value = height;
        onHeightChanged(height);
      });
    }

    useEffect(() {
      reportHeight();
      return null;
    }, const []);

    return NotificationListener<SizeChangedLayoutNotification>(
      onNotification: (_) {
        reportHeight();
        return true;
      },
      child: SizeChangedLayoutNotifier(
        child: KeyedSubtree(key: sizeKey, child: child),
      ),
    );
  }
}
