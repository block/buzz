import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Handles the native status-bar gesture for the visible page's vertical list.
class IosScrollToTop extends HookWidget {
  final Widget child;

  /// Used when a page coordinates multiple scroll positions, such as a title.
  final VoidCallback? onScrollToTop;

  const IosScrollToTop({super.key, required this.child, this.onScrollToTop});

  @override
  Widget build(BuildContext context) {
    final target = useMemoized(_ScrollTarget.new);
    final marker = useMemoized(GlobalKey.new);
    final position = useRef<ScrollPosition?>(null);
    final callback = useRef<VoidCallback>(() {});
    callback.value = () {
      if (!context.mounted || ModalRoute.of(context)?.isCurrent == false) {
        return;
      }
      final renderObject = marker.currentContext?.findRenderObject();
      if (renderObject == null || !renderObject.attached) return;
      final hits = HitTestResult();
      WidgetsBinding.instance.hitTestInView(
        hits,
        Offset.zero,
        View.of(context).viewId,
      );
      if (!hits.path.any((hit) => hit.target == renderObject)) return;
      if (target.callback != null) {
        target.callback!();
      } else if (onScrollToTop != null) {
        onScrollToTop!();
      } else {
        final scroll = position.value;
        if (scroll == null ||
            scroll.context.notificationContext?.mounted != true ||
            !scroll.hasContentDimensions) {
          return;
        }
        final top = scroll.axisDirection == AxisDirection.up
            ? scroll.maxScrollExtent
            : scroll.minScrollExtent;
        if (MediaQuery.disableAnimationsOf(context)) {
          scroll.jumpTo(top);
        } else {
          unawaited(
            scroll.animateTo(
              top,
              duration: const Duration(milliseconds: 300),
              curve: Curves.easeOutCubic,
            ),
          );
        }
      }
    };
    useEffect(() {
      if (defaultTargetPlatform != TargetPlatform.iOS) return null;
      final observer = _StatusBarObserver(() => callback.value());
      WidgetsBinding.instance.addObserver(observer);
      return () => WidgetsBinding.instance.removeObserver(observer);
    }, const []);
    if (defaultTargetPlatform != TargetPlatform.iOS) return child;
    return _ScrollTargetScope(
      target: target,
      child: Listener(
        key: marker,
        behavior: HitTestBehavior.translucent,
        child: NotificationListener<Notification>(
          onNotification: (notification) {
            final BuildContext? source;
            final int depth;
            if (notification is ScrollNotification) {
              source = notification.context;
              depth = notification.depth;
            } else if (notification is ScrollMetricsNotification) {
              source = notification.context;
              depth = notification.depth;
            } else {
              return false;
            }
            if (depth != 0 ||
                source == null ||
                source.findAncestorWidgetOfExactType<EditableText>() != null) {
              return false;
            }
            final scroll = Scrollable.maybeOf(source)?.position;
            if (scroll?.axis == Axis.vertical) position.value = scroll;
            return false;
          },
          // This page handles the native event, including custom/reversed lists.
          child: PrimaryScrollController.none(child: child),
        ),
      ),
    );
  }
}

/// Registers an indexed list's top target with its enclosing page.
void useIosScrollToTop(BuildContext context, VoidCallback callback) {
  final target = context
      .dependOnInheritedWidgetOfExactType<_ScrollTargetScope>()
      ?.target;
  useEffect(() {
    if (target == null) return null;
    target.callback = callback;
    return () {
      if (target.callback == callback) target.callback = null;
    };
  }, [target, callback]);
}

class _ScrollTarget {
  VoidCallback? callback;
}

class _ScrollTargetScope extends InheritedWidget {
  final _ScrollTarget target;
  const _ScrollTargetScope({required this.target, required super.child});
  @override
  bool updateShouldNotify(_ScrollTargetScope oldWidget) =>
      target != oldWidget.target;
}

class _StatusBarObserver with WidgetsBindingObserver {
  final VoidCallback onTap;
  _StatusBarObserver(this.onTap);
  @override
  void handleStatusBarTap() => onTap();
}
