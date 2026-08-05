import 'package:buzz/shared/widgets/concentric_sheet_surface.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/modal_presentation.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
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
        expect(closeButton, findsOneWidget);
        expect(tester.getSize(closeButton), const Size.square(44));
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
          isTrue,
        );
        expect(
          find.byKey(const ValueKey('buzz-sheet-drag-handle')),
          findsNothing,
        );
        expect(find.text('Sheet body'), findsOneWidget);

        await tester.tap(closeButton);
        await tester.pumpAndSettle();

        expect(find.text('Sheet body'), findsNothing);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

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
      expect(find.bySemanticsLabel('Drag handle'), findsOneWidget);
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
