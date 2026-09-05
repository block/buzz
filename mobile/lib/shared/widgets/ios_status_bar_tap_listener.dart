import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

Future<void> scrollPositionedListFromIosStatusBarTap({
  required BuildContext context,
  required ItemScrollController controller,
  required int targetIndex,
  VoidCallback? beforeScroll,
}) async {
  if (!controller.isAttached) return;
  beforeScroll?.call();
  if (MediaQuery.disableAnimationsOf(context)) {
    controller.jumpTo(index: targetIndex);
    return;
  }
  await controller.scrollTo(
    index: targetIndex,
    duration: const Duration(milliseconds: 500),
    curve: Curves.easeOutCubic,
    opacityAnimationWeights: const [20, 20, 60],
  );
}

/// Routes the native iOS status-bar scroll-to-top gesture to a custom
/// scrollable that cannot participate in Flutter's PrimaryScrollController.
class IosStatusBarTapListener extends HookWidget {
  const IosStatusBarTapListener({
    super.key,
    required this.onTap,
    required this.child,
  });

  final VoidCallback onTap;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final latestOnTap = useRef(onTap)..value = onTap;
    final route = ModalRoute.of(context);
    final observer = useMemoized(
      () => _IosStatusBarTapObserver(() {
        if (context.mounted && (route == null || route.isCurrent)) {
          latestOnTap.value();
        }
      }),
      [route],
    );

    useEffect(() {
      WidgetsBinding.instance.addObserver(observer);
      return () => WidgetsBinding.instance.removeObserver(observer);
    }, [observer]);

    return child;
  }
}

class _IosStatusBarTapObserver with WidgetsBindingObserver {
  _IosStatusBarTapObserver(this.onTap);

  final VoidCallback onTap;

  @override
  void handleStatusBarTap() => onTap();
}
