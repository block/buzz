import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// UIKit's measured navigation geometry, with a first-frame bootstrap size.
class IosNavigationMetrics {
  const IosNavigationMetrics({
    this.compactHeight = 44,
    this.largeTitleHeight = 52,
    this.railCenterX,
    this.windowSize,
    this.bookDivision,
  });

  final double compactHeight;
  final double largeTitleHeight;
  final double? railCenterX;
  final Size? windowSize;
  final Rect? bookDivision;

  /// Aligns the shared rail with the status cluster in the reserved region.
  double railCenter(Size size, double fallback) {
    final center = windowSize == size ? railCenterX ?? fallback : fallback;
    return center + (center < size.width / 2 ? 6 : -6);
  }

  static void updateRail(BuildContext context, Map<Object?, Object?> values) {
    final notifier = _MetricsScope.of(context);
    final center = (values['centerX'] as num?)?.toDouble();
    final width = (values['windowWidth'] as num?)?.toDouble();
    final height = (values['windowHeight'] as num?)?.toDouble();
    if (notifier == null ||
        center == null ||
        width == null ||
        height == null ||
        !center.isFinite ||
        !width.isFinite ||
        !height.isFinite ||
        center < 0 ||
        center > width ||
        width <= 0 ||
        height <= 0) {
      return;
    }
    final size = Size(width, height);
    final rawDivision = values['bookDivision'];
    Rect? division;
    if (rawDivision is List &&
        rawDivision.length == 4 &&
        rawDivision.every((value) => value is num && value.isFinite)) {
      final bounds = rawDivision
          .map((value) => (value as num).toDouble())
          .toList();
      final candidate = Rect.fromLTWH(
        bounds[0],
        bounds[1],
        bounds[2],
        bounds[3],
      );
      if (candidate.width >= 0 &&
          candidate.left > 0 &&
          candidate.right < width &&
          candidate.top <= 1 &&
          candidate.bottom >= height - 1) {
        division = candidate;
      }
    }
    final old = notifier.value;
    if (old.railCenterX == center &&
        old.windowSize == size &&
        old.bookDivision == division) {
      return;
    }
    notifier.value = IosNavigationMetrics(
      compactHeight: old.compactHeight,
      largeTitleHeight: old.largeTitleHeight,
      railCenterX: center,
      windowSize: size,
      bookDivision: division,
    );
  }

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
      railCenterX: notifier.value.railCenterX,
      windowSize: notifier.value.windowSize,
      bookDivision: notifier.value.bookDivision,
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
