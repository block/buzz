import 'package:buzz/shared/widgets/concentric_sheet_surface.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/modal_presentation.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  Border sheetHeaderBorder(WidgetTester tester) {
    final decoration =
        tester
                .widget<DecoratedBox>(
                  find.byKey(const ValueKey('buzz-sheet-scroll-divider')),
                )
                .decoration
            as BoxDecoration;
    return decoration.border! as Border;
  }

  testWidgets(
    'keeps an opaque Flutter surface when iOS native support is unavailable',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.light(),
            home: const ConcentricSheetSurface(
              enabled: true,
              color: Colors.red,
              child: SizedBox(height: 80, child: Text('Sheet body')),
            ),
          ),
        );
        await tester.pumpAndSettle();

        expect(
          find.byWidgetPredicate(
            (widget) => widget is Material && widget.color == Colors.red,
          ),
          findsOneWidget,
        );
        final contentClip = tester.widget<ClipRSuperellipse>(
          find.byKey(const ValueKey('concentric-sheet-content-clip')),
        );
        expect(contentClip.borderRadius, BorderRadius.circular(Radii.dialog));
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets(
    'replaces the Flutter fallback when native support is available',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      const surfaceChannel = MethodChannel('buzz/concentric_sheet_surface');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        surfaceChannel,
        (call) async => call.method == 'isSupported' ? true : null,
      );
      try {
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.light(),
            home: const ConcentricSheetSurface(
              enabled: true,
              color: Colors.red,
              child: SizedBox(height: 80, child: Text('Sheet body')),
            ),
          ),
        );
        await tester.pump();

        expect(find.byType(UiKitView), findsOneWidget);
        expect(
          find.ancestor(
            of: find.byType(UiKitView),
            matching: find.byWidgetPredicate(
              (widget) => widget is IgnorePointer && widget.ignoring,
            ),
          ),
          findsOneWidget,
        );
        expect(
          find.byWidgetPredicate(
            (widget) => widget is Material && widget.color == Colors.red,
          ),
          findsNothing,
        );
      } finally {
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          surfaceChannel,
          null,
        );
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets(
    'native surfaces can limit concentric clipping to bottom corners',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      const surfaceChannel = MethodChannel('buzz/concentric_sheet_surface');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        surfaceChannel,
        (call) async => call.method == 'isSupported' ? true : null,
      );
      try {
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.light(),
            home: const ConcentricSheetSurface(
              enabled: true,
              color: Colors.red,
              backdropColor: Colors.black,
              corners: ConcentricSurfaceCorners.bottom,
              padding: EdgeInsets.zero,
              providesSheetSurface: false,
              child: SizedBox(height: 80, child: Text('App surface')),
            ),
          ),
        );
        await tester.pump();

        final nativeSurface = tester.widget<UiKitView>(find.byType(UiKitView));
        expect(nativeSurface.creationParams, containsPair('corners', 'bottom'));
        expect(
          nativeSurface.creationParams,
          containsPair('backdropColor', Colors.black.toARGB32()),
        );
        final contentClip = tester.widget<ClipRSuperellipse>(
          find.byKey(const ValueKey('concentric-sheet-content-clip')),
        );
        expect(
          contentClip.borderRadius,
          BorderRadius.vertical(bottom: Radius.circular(Radii.dialog * 2)),
        );
      } finally {
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          surfaceChannel,
          null,
        );
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('native surface colors follow live theme changes', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const supportChannel = MethodChannel('buzz/concentric_sheet_surface');
    const viewChannel = MethodChannel('buzz/concentric_sheet_surface/42');
    final colorUpdates = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      supportChannel,
      (call) async => call.method == 'isSupported' ? true : null,
    );
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      viewChannel,
      (call) async {
        colorUpdates.add(call);
        return null;
      },
    );

    Widget themedSurface(ThemeData theme) => MaterialApp(
      theme: theme,
      home: Builder(
        builder: (context) => ConcentricSheetSurface(
          enabled: true,
          color: context.colors.surface,
          backdropColor: context.appColors.huddleDrawerSurface,
          corners: ConcentricSurfaceCorners.bottom,
          padding: EdgeInsets.zero,
          providesSheetSurface: false,
          child: const SizedBox(height: 80, child: Text('App surface')),
        ),
      ),
    );

    try {
      final darkTheme = AppTheme.dark();
      await tester.pumpWidget(themedSurface(darkTheme));
      await tester.pumpAndSettle();
      tester.widget<UiKitView>(find.byType(UiKitView)).onPlatformViewCreated!(
        42,
      );
      await tester.pump();

      final initialColorUpdates = colorUpdates
          .where((call) => call.method == 'updateColors')
          .toList();
      expect(initialColorUpdates, hasLength(1));
      expect(
        initialColorUpdates.single.arguments,
        containsPair('color', darkTheme.colorScheme.surface.toARGB32()),
      );
      expect(
        initialColorUpdates.single.arguments,
        containsPair(
          'backdropColor',
          darkTheme.extension<AppColors>()!.huddleDrawerSurface.toARGB32(),
        ),
      );

      final lightTheme = AppTheme.light();
      await tester.pumpWidget(themedSurface(lightTheme));
      await tester.pumpAndSettle();

      final updatedColorCalls = colorUpdates
          .where((call) => call.method == 'updateColors')
          .toList();
      expect(updatedColorCalls.length, greaterThanOrEqualTo(2));
      expect(
        updatedColorCalls.last.arguments,
        containsPair('color', lightTheme.colorScheme.surface.toARGB32()),
      );
      expect(
        updatedColorCalls.last.arguments,
        containsPair(
          'backdropColor',
          lightTheme.extension<AppColors>()!.huddleDrawerSurface.toARGB32(),
        ),
      );
    } finally {
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        supportChannel,
        null,
      );
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        viewChannel,
        null,
      );
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('native glass surfaces receive concentric composer geometry', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const supportChannel = MethodChannel('buzz/concentric_sheet_surface');
    const viewChannel = MethodChannel('buzz/concentric_sheet_surface/84');
    final updates = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      supportChannel,
      (call) async => call.method == 'isSupported' ? true : null,
    );
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      viewChannel,
      (call) async {
        updates.add(call);
        return null;
      },
    );
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: const ConcentricSheetSurface(
            enabled: true,
            usesGlass: true,
            minimumRadius: 26,
            contentClipRadius: 18,
            padding: EdgeInsets.zero,
            providesSheetSurface: false,
            child: SizedBox(height: 80, child: Text('Composer')),
          ),
        ),
      );
      await tester.pump();

      final nativeSurface = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(nativeSurface.creationParams, containsPair('usesGlass', true));
      expect(nativeSurface.creationParams, containsPair('minimumRadius', 26));
      final contentClip = tester.widget<ClipRSuperellipse>(
        find.byKey(const ValueKey('concentric-sheet-content-clip')),
      );
      expect(contentClip.borderRadius, BorderRadius.circular(18));

      nativeSurface.onPlatformViewCreated!(84);
      await tester.pump();
      final geometryUpdate = updates.singleWhere(
        (call) => call.method == 'updateGeometry',
      );
      expect(geometryUpdate.arguments, containsPair('minimumRadius', 26.0));
      expect(geometryUpdate.arguments, containsPair('brightness', 'light'));
    } finally {
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        supportChannel,
        null,
      );
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        viewChannel,
        null,
      );
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('native titled sheets leave the concentric surface unobscured', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const surfaceChannel = MethodChannel('buzz/concentric_sheet_surface');
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      surfaceChannel,
      (call) async => call.method == 'isSupported' ? true : null,
    );
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Builder(
              builder: (context) => FilledButton(
                onPressed: () => showBuzzModalBottomSheet<void>(
                  context: context,
                  title: 'Members',
                  builder: (sheetContext) => ColoredBox(
                    key: const ValueKey('sheet-container-surface'),
                    color: Theme.of(
                      sheetContext,
                    ).colorScheme.surfaceContainerHighest,
                    child: const Text('Sheet body'),
                  ),
                ),
                child: const Text('Open sheet'),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Open sheet'));
      await tester.pumpAndSettle();

      final nativeSurface = tester.widget<UiKitView>(
        find.byWidgetPredicate(
          (widget) =>
              widget is UiKitView &&
              widget.viewType == 'buzz/concentric_sheet_surface',
        ),
      );
      expect(
        nativeSurface.creationParams,
        containsPair(
          'color',
          lightColorScheme.surfaceContainerHighest.toARGB32(),
        ),
      );
      expect(
        tester
            .widget<ColoredBox>(
              find.byKey(const ValueKey('sheet-container-surface')),
            )
            .color,
        lightColorScheme.surface,
      );
      expect(nativeSurface.creationParams, isNot(contains('headerGradient')));
      expect(
        find.byKey(const ValueKey('buzz-sheet-header-gradient')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('buzz-sheet-surface-clip')),
        findsNothing,
      );
      final contentClip = tester.widget<ClipRSuperellipse>(
        find.byKey(const ValueKey('concentric-sheet-content-clip')),
      );
      expect(contentClip.borderRadius, BorderRadius.circular(Radii.dialog * 2));
      expect(contentClip.clipBehavior, Clip.antiAlias);
      expect(find.text('Members'), findsOneWidget);
      final nativeClose = tester.widget<UiKitView>(
        find.byWidgetPredicate(
          (widget) =>
              widget is UiKitView && widget.viewType == 'buzz/navigation_glass',
        ),
      );
      expect(nativeClose.creationParams, containsPair('icon', 'close'));
    } finally {
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        surfaceChannel,
        null,
      );
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets(
    'non-iOS sheets use Flutter drag handle and shared close control',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      try {
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: Builder(
                builder: (context) => FilledButton(
                  onPressed: () => showBuzzModalBottomSheet<void>(
                    context: context,
                    title: 'Sheet title',
                    showDragHandle: true,
                    builder: (_) => const Text('Sheet body'),
                  ),
                  child: const Text('Open sheet'),
                ),
              ),
            ),
          ),
        );

        await tester.tap(find.text('Open sheet'));
        await tester.pumpAndSettle();

        final closeButton = find.byTooltip('Close sheet');
        final title = find.byKey(const ValueKey('buzz-sheet-title'));
        expect(closeButton, findsOneWidget);
        expect(title, findsOneWidget);
        expect(
          find.byKey(const ValueKey('buzz-sheet-surface-clip')),
          findsOneWidget,
        );
        expect(tester.getSize(closeButton), const Size.square(44));
        expect(tester.widget<Text>(title).style?.fontSize, 16);
        expect(
          find.byKey(const ValueKey('buzz-sheet-header-gradient')),
          findsNothing,
        );
        expect(
          tester
              .widget<ColoredBox>(
                find.byKey(const ValueKey('buzz-sheet-surface')),
              )
              .color,
          lightColorScheme.surfaceContainerHighest,
        );
        expect(
          tester.getTopLeft(find.text('Sheet body')).dy -
              tester
                  .getTopLeft(find.byKey(const ValueKey('buzz-sheet-surface')))
                  .dy,
          80,
        );
        expect(find.byType(BackdropFilter), findsNothing);
        expect(
          tester.widget<BottomSheet>(find.byType(BottomSheet)).backgroundColor,
          Colors.transparent,
        );
        expect(
          tester.getCenter(title).dx,
          closeTo(tester.getCenter(find.byType(BottomSheet)).dx, 0.01),
        );
        expect(
          tester.getCenter(title).dy,
          closeTo(tester.getCenter(closeButton).dy, 0.01),
        );
        expect(
          tester.getCenter(closeButton).dx,
          greaterThan(tester.getCenter(title).dx),
        );
        final closeGutter = find.ancestor(
          of: closeButton,
          matching: find.byWidgetPredicate(
            (widget) =>
                widget is Padding &&
                widget.padding ==
                    const EdgeInsets.only(
                      top: Grid.xxs,
                      left: Grid.gutter,
                      right: Grid.gutter,
                      bottom: Grid.xs,
                    ),
          ),
        );
        expect(closeGutter, findsOneWidget);
        final gutterRect = tester.getRect(closeGutter);
        final closeRect = tester.getRect(closeButton);
        expect(closeRect.top - gutterRect.top, Grid.gutter);
        expect(gutterRect.right - closeRect.right, Grid.gutter);
        expect(
          tester.widget<BottomSheet>(find.byType(BottomSheet)).showDragHandle,
          isFalse,
        );
        expect(
          find.byKey(const ValueKey('buzz-sheet-drag-handle')),
          findsOneWidget,
        );
        expect(
          tester.getTopLeft(closeButton).dy -
              tester.getTopLeft(find.byType(BottomSheet)).dy,
          Grid.gutter,
        );
        final dismissHandle = find.bySemanticsLabel('Dismiss').first;
        expect(dismissHandle, findsOneWidget);
        final semantics = tester.getSemantics(dismissHandle);
        expect(semantics.flagsCollection.isButton, isTrue);
        expect(
          semantics.getSemanticsData().hasAction(SemanticsAction.tap),
          isTrue,
        );
        tester.binding.performSemanticsAction(
          SemanticsActionEvent(
            type: SemanticsAction.tap,
            viewId: tester.view.viewId,
            nodeId: semantics.id,
          ),
        );
        await tester.pumpAndSettle();
        expect(find.text('Sheet body'), findsNothing);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('untitled Android sheets paint the utility route surface', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Builder(
              builder: (context) => FilledButton(
                onPressed: () => showBuzzModalBottomSheet<void>(
                  context: context,
                  builder: (sheetContext) => ColoredBox(
                    key: const ValueKey('untitled-sheet-container'),
                    color: Theme.of(
                      sheetContext,
                    ).colorScheme.surfaceContainerHighest,
                    child: const SizedBox(
                      height: 80,
                      child: Text('Sheet body'),
                    ),
                  ),
                ),
                child: const Text('Open sheet'),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Open sheet'));
      await tester.pumpAndSettle();

      expect(
        tester.widget<BottomSheet>(find.byType(BottomSheet)).backgroundColor,
        lightColorScheme.surfaceContainerHighest,
      );
      expect(
        tester
            .widget<ColoredBox>(
              find.byKey(const ValueKey('untitled-sheet-container')),
            )
            .color,
        lightColorScheme.surface,
      );
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('sheet divider appears only when content scrolls under header', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Builder(
              builder: (context) => FilledButton(
                onPressed: () => showBuzzModalBottomSheet<void>(
                  context: context,
                  title: 'Theme',
                  isScrollControlled: true,
                  builder: (_) => SizedBox(
                    height: 360,
                    child: ListView.builder(
                      key: const ValueKey('sheet-scroll-view'),
                      itemCount: 30,
                      itemBuilder: (_, index) =>
                          SizedBox(height: 44, child: Text('Option $index')),
                    ),
                  ),
                ),
                child: const Text('Open sheet'),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Open sheet'));
      await tester.pumpAndSettle();

      expect(sheetHeaderBorder(tester).bottom.color.a, 0);

      await tester.drag(
        find.byKey(const ValueKey('sheet-scroll-view')),
        const Offset(0, -100),
      );
      await tester.pumpAndSettle();

      expect(
        tester
            .state<ScrollableState>(
              find.descendant(
                of: find.byKey(const ValueKey('sheet-scroll-view')),
                matching: find.byType(Scrollable),
              ),
            )
            .position
            .pixels,
        greaterThan(0),
      );
      expect(sheetHeaderBorder(tester).bottom.color.a, greaterThan(0));

      await tester.drag(
        find.byKey(const ValueKey('sheet-scroll-view')),
        const Offset(0, 500),
      );
      await tester.pumpAndSettle();

      expect(sheetHeaderBorder(tester).bottom.color.a, 0);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('iOS paints the drag handle inside the concentric surface', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Builder(
              builder: (context) => FilledButton(
                onPressed: () => showBuzzModalBottomSheet<void>(
                  context: context,
                  showDragHandle: true,
                  builder: (_) => const Text('Sheet body'),
                ),
                child: const Text('Open sheet'),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Open sheet'));
      await tester.pumpAndSettle();

      expect(
        tester.widget<BottomSheet>(find.byType(BottomSheet)).showDragHandle,
        isFalse,
      );
      final internalHandle = find.byKey(
        const ValueKey('buzz-sheet-drag-handle'),
      );
      expect(internalHandle, findsOneWidget);
      expect(tester.getSize(internalHandle), const Size(32, 4));
      final dismissHandle = find.bySemanticsLabel('Dismiss');
      expect(dismissHandle, findsOneWidget);
      final semantics = tester.getSemantics(dismissHandle);
      expect(semantics.flagsCollection.isButton, isTrue);
      expect(
        semantics.getSemanticsData().hasAction(SemanticsAction.tap),
        isTrue,
      );
      expect(find.byTooltip('Close sheet'), findsOneWidget);
      expect(find.text('Sheet body'), findsOneWidget);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('iOS compact sheets can omit X but retain the inside handle', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Builder(
              builder: (context) => FilledButton(
                onPressed: () => showBuzzModalBottomSheet<void>(
                  context: context,
                  showDragHandle: true,
                  showCloseButton: false,
                  builder: (_) => const Text('Compact sheet body'),
                ),
                child: const Text('Open compact sheet'),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Open compact sheet'));
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('buzz-sheet-drag-handle')),
        findsOneWidget,
      );
      expect(find.byTooltip('Close sheet'), findsNothing);
      expect(find.text('Compact sheet body'), findsOneWidget);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
