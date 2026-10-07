part of '../thread_detail_page.dart';

/// Reports the persistent head's extent for its placeholder in the lazy list.
class _ThreadHeadLayout extends HookWidget {
  final ValueChanged<double> onHeightChanged;
  final Widget child;

  const _ThreadHeadLayout({required this.onHeightChanged, required this.child});

  @override
  Widget build(BuildContext context) {
    final sizeKey = useMemoized(GlobalKey.new);
    final lastHeight = useRef<double?>(null);
    void reportHeight() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        final box = sizeKey.currentContext?.findRenderObject();
        if (box is! RenderBox || !box.hasSize) return;
        final height = box.size.height;
        if (lastHeight.value == height) return;
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
