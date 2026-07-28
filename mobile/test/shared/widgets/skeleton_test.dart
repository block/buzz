import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/skeleton.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  Widget buildShimmerTestable({
    bool disableAnimations = false,
    bool enabled = true,
  }) {
    return MaterialApp(
      theme: AppTheme.light(),
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(
          context,
        ).copyWith(disableAnimations: disableAnimations),
        child: child!,
      ),
      home: Scaffold(
        body: SkeletonShimmer(
          enabled: enabled,
          child: const SkeletonBar(width: 120, height: 16),
        ),
      ),
    );
  }

  Widget buildRevealTestable({
    required ValueNotifier<bool> loading,
    bool disableAnimations = false,
  }) {
    return MaterialApp(
      theme: AppTheme.light(),
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(
          context,
        ).copyWith(disableAnimations: disableAnimations),
        child: child!,
      ),
      home: Scaffold(
        body: ValueListenableBuilder(
          valueListenable: loading,
          builder: (context, isLoading, _) => SkeletonReveal(
            loading: isLoading,
            skeleton: const SkeletonBar(width: 120, height: 16),
            content: const Text('Loaded content'),
          ),
        ),
      ),
    );
  }

  double layerOpacity(WidgetTester tester, String key) =>
      tester.widget<Opacity>(find.byKey(Key(key))).opacity;

  bool contentFocusExcluded(WidgetTester tester) => tester
      .widget<ExcludeFocus>(
        find.descendant(
          of: find.byKey(const Key('skeleton-reveal-content')),
          matching: find.byType(ExcludeFocus),
        ),
      )
      .excluding;

  testWidgets('sweeps a highlight across skeleton elements while loading', (
    tester,
  ) async {
    await tester.pumpWidget(buildShimmerTestable());

    final shimmer = find.byType(SkeletonShimmer);
    expect(find.byType(SkeletonBar), findsOneWidget);
    expect(
      find.descendant(of: shimmer, matching: find.byType(ShaderMask)),
      findsOneWidget,
    );

    final boundary = tester.renderObject<RenderRepaintBoundary>(
      find
          .descendant(of: shimmer, matching: find.byType(RepaintBoundary))
          .first,
    );
    final beforeBytes = await tester.runAsync(() async {
      final image = await boundary.toImage();
      return image.toByteData();
    });

    await tester.pump(const Duration(milliseconds: 1000));
    final afterBytes = await tester.runAsync(() async {
      final image = await boundary.toImage();
      return image.toByteData();
    });

    expect(
      beforeBytes!.buffer.asUint8List(),
      isNot(equals(afterBytes!.buffer.asUint8List())),
    );
  });

  testWidgets('keeps skeleton elements static for reduced motion', (
    tester,
  ) async {
    await tester.pumpWidget(buildShimmerTestable(disableAnimations: true));

    final shimmer = find.byType(SkeletonShimmer);
    expect(find.byType(SkeletonBar), findsOneWidget);
    expect(
      find.descendant(of: shimmer, matching: find.byType(ShaderMask)),
      findsNothing,
    );
  });

  testWidgets('keeps skeleton elements static when shimmer is disabled', (
    tester,
  ) async {
    await tester.pumpWidget(buildShimmerTestable(enabled: false));

    final shimmer = find.byType(SkeletonShimmer);
    expect(find.byType(SkeletonBar), findsOneWidget);
    expect(
      find.descendant(of: shimmer, matching: find.byType(ShaderMask)),
      findsNothing,
    );
  });

  testWidgets('reveals content in the same slot with a 400ms cross-fade', (
    tester,
  ) async {
    final loading = ValueNotifier(true);
    await tester.pumpWidget(buildRevealTestable(loading: loading));

    expect(layerOpacity(tester, 'skeleton-reveal-placeholder'), 1);
    expect(layerOpacity(tester, 'skeleton-reveal-content'), 0);
    expect(contentFocusExcluded(tester), isTrue);

    loading.value = false;
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 200));

    expect(
      layerOpacity(tester, 'skeleton-reveal-placeholder'),
      closeTo(0.5, 0.01),
    );
    expect(layerOpacity(tester, 'skeleton-reveal-content'), closeTo(0.5, 0.01));
    expect(contentFocusExcluded(tester), isTrue);

    await tester.pump(const Duration(milliseconds: 200));

    expect(layerOpacity(tester, 'skeleton-reveal-placeholder'), 0);
    expect(layerOpacity(tester, 'skeleton-reveal-content'), 1);
    expect(contentFocusExcluded(tester), isFalse);
    expect(find.text('Loaded content'), findsOneWidget);
  });

  testWidgets('resets to the skeleton without animating backwards', (
    tester,
  ) async {
    final loading = ValueNotifier(false);
    await tester.pumpWidget(buildRevealTestable(loading: loading));
    expect(layerOpacity(tester, 'skeleton-reveal-content'), 1);

    loading.value = true;
    await tester.pump();

    expect(layerOpacity(tester, 'skeleton-reveal-placeholder'), 1);
    expect(layerOpacity(tester, 'skeleton-reveal-content'), 0);
  });

  testWidgets('reveals immediately for reduced motion', (tester) async {
    final loading = ValueNotifier(true);
    await tester.pumpWidget(
      buildRevealTestable(loading: loading, disableAnimations: true),
    );
    final reveal = find.byType(SkeletonReveal);
    expect(
      find.descendant(of: reveal, matching: find.byType(ShaderMask)),
      findsNothing,
    );

    loading.value = false;
    await tester.pump();

    expect(layerOpacity(tester, 'skeleton-reveal-placeholder'), 0);
    expect(layerOpacity(tester, 'skeleton-reveal-content'), 1);
  });
}
