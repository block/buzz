import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// UIKit's measured navigation geometry, with a first-frame bootstrap size.
class IosNavigationMetrics {
  const IosNavigationMetrics({
    this.compactHeight = 44,
    this.largeTitleHeight = 52,
  });

  final double compactHeight;
  final double largeTitleHeight;

  static IosNavigationMetrics of(BuildContext context) =>
      _MetricsScope.of(context)?.value ?? const IosNavigationMetrics();

  static void update(BuildContext context, Map<Object?, Object?> values) {
    final notifier = _MetricsScope.of(context);
    if (notifier == null) return;
    final compact = (values['compactHeight'] as num?)?.toDouble();
    final expanded = (values['expandedHeight'] as num?)?.toDouble();
    if (compact == null || !compact.isFinite || compact <= 0) return;
    final extra = expanded == null
        ? notifier.value.largeTitleHeight
        : expanded - compact;
    if (!extra.isFinite || extra < 0) return;
    if ((notifier.value.compactHeight - compact).abs() < 0.1 &&
        (notifier.value.largeTitleHeight - extra).abs() < 0.1) {
      return;
    }
    notifier.value = IosNavigationMetrics(
      compactHeight: compact,
      largeTitleHeight: extra,
    );
  }
}

/// Shares measured UIKit spacing across routes and their body insets.
class IosNavigationMetricsHost extends HookWidget {
  const IosNavigationMetricsHost({super.key, required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final metrics = useValueNotifier(const IosNavigationMetrics());
    return _MetricsScope(notifier: metrics, child: child);
  }
}

class _MetricsScope
    extends InheritedNotifier<ValueNotifier<IosNavigationMetrics>> {
  const _MetricsScope({required super.notifier, required super.child});

  static ValueNotifier<IosNavigationMetrics>? of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_MetricsScope>()?.notifier;
}
