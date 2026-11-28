import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// UIKit's measured navigation geometry, with a first-frame bootstrap size.
class IosNavigationMetrics {
  const IosNavigationMetrics({
    this.compactHeight = 44,
    this.largeTitleHeight = 52,
    this.railCenterX,
    this.railTopClearance,
    this.windowSize,
    this.bookDivision,
    this.portraitHeader,
    this.portraitWindowSize,
    this.laptopDivision,
    this.laptopWindowSize,
  });

  final double compactHeight;
  final double largeTitleHeight;
  final double? railCenterX;
  final double? railTopClearance;
  final Size? windowSize;
  final Rect? bookDivision;
  final Rect? portraitHeader;
  final Size? portraitWindowSize;

  /// A fold across the window, as when the device is half open like a laptop,
  /// measured in a window of [laptopWindowSize].
  final Rect? laptopDivision;
  final Size? laptopWindowSize;

  /// The laptop fold, when it was measured in a window of [size].
  Rect? laptopDivisionFor(Size size) =>
      laptopWindowSize == size ? laptopDivision : null;

  Rect? portraitHeaderFor(Size size) =>
      portraitWindowSize == size ? portraitHeader : null;

  static void updatePortraitHeader(BuildContext context, List<Object?> values) {
    final notifier = _MetricsScope.of(context);
    if (notifier == null ||
        values.length != 6 ||
        !values.every((v) => v is num && v.isFinite)) {
      return;
    }
    final v = values.cast<num>().map((n) => n.toDouble()).toList();
    if (v[0] <= 0 || v[1] <= 0) return;
    final size = Size(v[0], v[1]);
    final rect = Rect.fromLTWH(v[2], v[3], v[4], v[5]);
    final header = rect.isEmpty ? null : rect;
    if (header != null &&
        (header.left < 0 ||
            header.top < 0 ||
            header.right > size.width ||
            header.bottom > size.height)) {
      return;
    }
    final old = notifier.value;
    if (old.portraitWindowSize == size && old.portraitHeader == header) return;
    notifier.value = IosNavigationMetrics(
      compactHeight: old.compactHeight,
      largeTitleHeight: old.largeTitleHeight,
      railCenterX: old.railCenterX,
      railTopClearance: old.railTopClearance,
      windowSize: old.windowSize,
      bookDivision: old.bookDivision,
      portraitHeader: header,
      laptopDivision: old.laptopDivision,
      laptopWindowSize: old.laptopWindowSize,
      portraitWindowSize: size,
    );
  }

  /// Records the laptop fold UIKit reports for the current window.
  static void updateLaptop(BuildContext context, Map<Object?, Object?> values) {
    final notifier = _MetricsScope.of(context);
    final width = (values['windowWidth'] as num?)?.toDouble();
    final height = (values['windowHeight'] as num?)?.toDouble();
    if (notifier == null ||
        width == null ||
        height == null ||
        !width.isFinite ||
        !height.isFinite ||
        width <= 0 ||
        height <= 0) {
      return;
    }
    final size = Size(width, height);
    final raw = values['division'];
    Rect? division;
    if (raw is List &&
        raw.length == 4 &&
        raw.every((value) => value is num && value.isFinite)) {
      final v = raw.map((value) => (value as num).toDouble()).toList();
      final candidate = Rect.fromLTWH(v[0], v[1], v[2], v[3]);
      // Only a fold that crosses the whole window, with room on both sides.
      if (candidate.height >= 0 &&
          candidate.left <= 1 &&
          candidate.right >= width - 1 &&
          candidate.top > 0 &&
          candidate.bottom < height) {
        division = candidate;
      }
    }
    final old = notifier.value;
    if (old.laptopWindowSize == size && old.laptopDivision == division) return;
    notifier.value = IosNavigationMetrics(
      compactHeight: old.compactHeight,
      largeTitleHeight: old.largeTitleHeight,
      railCenterX: old.railCenterX,
      railTopClearance: old.railTopClearance,
      windowSize: old.windowSize,
      bookDivision: old.bookDivision,
      portraitHeader: old.portraitHeader,
      portraitWindowSize: old.portraitWindowSize,
      laptopDivision: division,
      laptopWindowSize: size,
    );
  }

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
    final rawTop = (values['topClearance'] as num?)?.toDouble();
    final top =
        rawTop != null && rawTop.isFinite && rawTop >= 0 && rawTop < height
        ? rawTop
        : null;
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
        old.railTopClearance == top &&
        old.windowSize == size &&
        old.bookDivision == division) {
      return;
    }
    notifier.value = IosNavigationMetrics(
      compactHeight: old.compactHeight,
      largeTitleHeight: old.largeTitleHeight,
      railCenterX: center,
      railTopClearance: top,
      windowSize: size,
      bookDivision: division,
      portraitHeader: old.portraitHeader,
      laptopDivision: old.laptopDivision,
      laptopWindowSize: old.laptopWindowSize,
      portraitWindowSize: old.portraitWindowSize,
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
      railTopClearance: notifier.value.railTopClearance,
      windowSize: notifier.value.windowSize,
      bookDivision: notifier.value.bookDivision,
      portraitHeader: notifier.value.portraitHeader,
      laptopDivision: notifier.value.laptopDivision,
      laptopWindowSize: notifier.value.laptopWindowSize,
      portraitWindowSize: notifier.value.portraitWindowSize,
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
