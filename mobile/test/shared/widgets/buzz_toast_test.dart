import 'package:buzz/shared/clipboard_utils.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_toast.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'iPhone toast automatically expands from the island and returns',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.padding = FakeViewPadding(top: 59);
      tester.view.viewPadding = FakeViewPadding(top: 59);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      const origin = Rect.fromLTWH(135, 11, 120, 36);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () => showBuzzToast(context, 'Island preview'),
                child: const Text('Show'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Show'));
      await tester.pump();
      final surface = find.ancestor(
        of: find.text('Island preview'),
        matching: find.byType(PhysicalShape),
      );
      // Shader readiness may span frames. No fallback circle is painted while
      // waiting, and animation time does not advance before the surface exists.
      for (var frame = 0; frame < 20 && surface.evaluate().isEmpty; frame++) {
        expect(find.text('Island preview'), findsNothing);
        await tester.pump();
      }
      expect(tester.widget<PhysicalShape>(surface).color, Colors.transparent);
      expect(tester.getRect(surface).center.dy, closeTo(origin.center.dy, 1));
      await tester.pump(const Duration(milliseconds: 140));
      expect(tester.getRect(surface).center.dy, greaterThan(origin.center.dy));
      await tester.pumpAndSettle();
      // The shader owns the unified background; no opaque second capsule or
      // independent shadow may divide its silhouette.
      expect(tester.widget<PhysicalShape>(surface).color, Colors.transparent);
      expect(tester.widget<PhysicalShape>(surface).elevation, 0);
      await tester.pump(const Duration(seconds: 3));
      await tester.pump(const Duration(milliseconds: 120));
      expect(tester.getRect(surface).top, lessThan(67));
      await tester.pumpAndSettle();
      expect(find.text('Island preview'), findsNothing);
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets('Retry capsule stays inset and invokes the action once', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(320, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    var retries = 0;
    for (final brightness in Brightness.values) {
      await tester.pumpWidget(
        MaterialApp(
          home: MediaQuery(
            data: MediaQueryData(
              platformBrightness: brightness,
              accessibleNavigation: true,
              disableAnimations: true,
              textScaler: TextScaler.linear(2),
            ),
            child: Scaffold(
              body: Builder(
                builder: (context) => TextButton(
                  onPressed: () => showBuzzToast(
                    context,
                    'Couldn’t save changes',
                    kind: ToastKind.error,
                    onRetry: () => retries++,
                  ),
                  child: const Text('Show error'),
                ),
              ),
            ),
          ),
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(context).copyWith(
              platformBrightness: brightness,
              accessibleNavigation: true,
              disableAnimations: true,
              textScaler: TextScaler.linear(2),
            ),
            child: child!,
          ),
        ),
      );
      await tester.tap(find.text('Show error'));
      await tester.pumpAndSettle();
      final button = find.widgetWithText(FilledButton, 'Retry');
      final shape = find
          .ancestor(of: button, matching: find.byType(PhysicalShape))
          .first;
      final outer = tester.getRect(shape);
      final inner = tester.getRect(button);
      expect(inner.top - outer.top, 6);
      expect(outer.right - inner.right, 6);
      expect(outer.bottom - inner.bottom, 6);
      final style = tester.widget<FilledButton>(button).style!;
      expect(style.backgroundColor!.resolve({}), Colors.white);
      expect(style.foregroundColor!.resolve({}), Colors.black);
      await tester.pump(const Duration(seconds: 20));
      expect(button, findsOneWidget);
      await tester.tap(button);
      await tester.pump();
      expect(button, findsNothing);
      expect(tester.takeException(), isNull);
    }
    expect(retries, 2);
  });

  testWidgets(
    'toast kinds request matching platform feedback',
    (tester) async {
      final nativeCalls = <String>[];
      final impactCalls = <String>[];
      final messenger = tester.binding.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(const MethodChannel('buzz/haptics'), (
        call,
      ) async {
        nativeCalls.add(call.method);
        return null;
      });
      messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
        if (call.method == 'HapticFeedback.vibrate') {
          impactCalls.add(call.arguments as String);
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(
          const MethodChannel('buzz/haptics'),
          null,
        );
        messenger.setMockMethodCallHandler(SystemChannels.platform, null);
      });
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Builder(
              builder: (context) {
                return Column(
                  children: [
                    for (final kind in ToastKind.values)
                      TextButton(
                        onPressed: () =>
                            showBuzzToast(context, kind.name, kind: kind),
                        child: Text('Show ${kind.name}'),
                      ),
                  ],
                );
              },
            ),
          ),
        ),
      );
      for (final kind in ToastKind.values) {
        await tester.tap(find.text('Show ${kind.name}'));
        await tester.pumpAndSettle();
        expect(find.text(kind.name), findsOneWidget);
      }
      if (Theme.of(tester.element(find.byType(Scaffold))).platform ==
          TargetPlatform.iOS) {
        expect(nativeCalls, ['success', 'error']);
        expect(impactCalls, ['HapticFeedbackType.lightImpact']);
      } else {
        expect(nativeCalls, isEmpty);
        expect(impactCalls, [
          'HapticFeedbackType.lightImpact',
          'HapticFeedbackType.mediumImpact',
          'HapticFeedbackType.heavyImpact',
        ]);
      }
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant({
      TargetPlatform.iOS,
      TargetPlatform.android,
    }),
  );

  testWidgets('copy feedback floats above sheets, replaces, and exits upward', (
    tester,
  ) async {
    var hapticCount = 0;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'HapticFeedback.vibrate') {
          expect(call.arguments, 'HapticFeedbackType.mediumImpact');
          hapticCount++;
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    tester.view.viewPadding = const FakeViewPadding(top: 59, bottom: 34);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Builder(
            builder: (context) {
              return Center(
                child: TextButton(
                  onPressed: () => showModalBottomSheet<void>(
                    context: context,
                    builder: (context) => SizedBox(
                      height: 200,
                      child: TextButton(
                        onPressed: () => copyToClipboard(context, 'example'),
                        child: const Text('Copy'),
                      ),
                    ),
                  ),
                  child: const Text('Open sheet'),
                ),
              );
            },
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open sheet'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Copy'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 140));
    expect(hapticCount, 1);
    final enteringY = tester.getTopLeft(find.text('Copied')).dy;
    final textWidth = tester.getSize(find.text('Copied')).width;
    final shapeFinder = find
        .ancestor(of: find.text('Copied'), matching: find.byType(PhysicalShape))
        .first;
    final shape = tester.widget<PhysicalShape>(shapeFinder);
    final shapeSize = tester.getSize(shapeFinder);
    final revealingWidth = shape.clipper.getClip(shapeSize).getBounds().width;
    expect(revealingWidth, lessThan(shapeSize.width));

    await tester.pump(const Duration(milliseconds: 160));
    final settledY = tester.getTopLeft(find.text('Copied')).dy;
    expect((settledY - enteringY).abs(), lessThan(5));
    expect(tester.getSize(find.text('Copied')).width, textWidth);
    expect(
      tester
          .widget<PhysicalShape>(shapeFinder)
          .clipper
          .getClip(shapeSize)
          .getBounds()
          .width,
      shapeSize.width,
    );

    expect(settledY, greaterThan(59));
    expect(settledY, lessThan(120));
    expect(tester.getSize(find.text('Copied')).width, lessThan(350));
    expect(find.byType(SnackBar), findsNothing);
    final toastMaterial = find
        .ancestor(of: find.text('Copied'), matching: find.byType(PhysicalShape))
        .first;
    expect(
      tester.widget<PhysicalShape>(toastMaterial).color,
      lightColorScheme.inverseSurface,
    );

    // Feedback remains above the sheet, and a new copy restarts its lifetime.
    await tester.pump(const Duration(seconds: 2));
    await tester.tap(find.text('Copy'));
    await tester.pumpAndSettle();
    expect(find.text('Copied'), findsOneWidget);
    expect(hapticCount, 2);
    await tester.pump(const Duration(seconds: 1));
    expect(find.text('Copied'), findsOneWidget);
    await tester.pump(const Duration(seconds: 2));
    await tester.pump(const Duration(milliseconds: 40));
    final exitingShape = tester.widget<PhysicalShape>(shapeFinder);
    expect(
      exitingShape.clipper.getClip(shapeSize).getBounds().width,
      lessThan(shapeSize.width * 0.9),
    );
    final opacity = tester.widget<Opacity>(
      find
          .ancestor(of: find.text('Copied'), matching: find.byType(Opacity))
          .first,
    );
    expect(opacity.opacity, lessThan(0.8));
    expect(tester.getSize(find.text('Copied')).width, textWidth);
    expect(tester.getTopLeft(find.text('Copied')).dy, lessThan(settledY));
    await tester.pump(const Duration(milliseconds: 220));
    expect(find.text('Copied'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'system dark mode, reduced motion, and large text are supported',
    (tester) async {
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
      addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(context).copyWith(
              disableAnimations: true,
              textScaler: const TextScaler.linear(2),
            ),
            child: child!,
          ),
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () => showBuzzToast(context, 'Channel name copied'),
                child: const Text('Copy'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Copy'));
      await tester.pump();
      final text = find.text('Channel name copied');
      final y = tester.getTopLeft(text).dy;
      await tester.pump(const Duration(milliseconds: 150));
      expect(tester.getTopLeft(text).dy, y);
      final material = find
          .ancestor(of: text, matching: find.byType(PhysicalShape))
          .first;
      expect(
        tester.widget<PhysicalShape>(material).color,
        darkColorScheme.surfaceContainer,
      );
      expect(tester.takeException(), isNull);
      await tester.pump(const Duration(seconds: 3));
      expect(text, findsNothing);
      // Removing the app while a toast is alive must cancel its timer.
      await tester.tap(find.text('Copy'));
      await tester.pump();
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pump(const Duration(seconds: 4));
      expect(tester.takeException(), isNull);
    },
  );
}
