import 'package:buzz/features/pairing/pairing_qr_scanner.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _qrScannerPlatformChannel = MethodChannel('buzz/qr_scanner');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() async {
    debugDefaultTargetPlatformOverride = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_qrScannerPlatformChannel, null);
  });

  group('DynamicIslandQrScannerGeometry', () {
    const viewport = Size(393, 852);
    const geometry = DynamicIslandQrScannerGeometry(
      viewport: viewport,
      safeAreaTop: 59,
    );

    test('starts at the physical Dynamic Island frame', () {
      expect(geometry.collapsedFrame, const Rect.fromLTWH(136.5, 11, 120, 36));
    });

    test('keeps the top edge fixed while the camera grows down', () {
      final start = geometry.frameAt(0);
      final middle = geometry.frameAt(0.5);
      final end = geometry.frameAt(1);

      expect(middle.top, start.top);
      expect(end.top, start.top);
      expect(middle.bottom, greaterThan(start.bottom));
      expect(end.bottom, greaterThan(middle.bottom));
      expect(end, const Rect.fromLTWH(15, 11, 363, 363));
    });

    test('delays the camera until the portal has left the island', () {
      expect(geometry.scannerOpacityAt(0.18), 0);
      expect(geometry.scannerOpacityAt(0.5), greaterThan(0));
      expect(geometry.scannerOpacityAt(1), 1);
    });
  });

  group('usesDynamicIslandQrScannerPortal', () {
    test('asks the native bridge on iOS', () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_qrScannerPlatformChannel, (call) async {
            expect(call.method, 'usesDynamicIslandQrScannerPortal');
            return true;
          });

      expect(await usesDynamicIslandQrScannerPortal(), isTrue);
    });

    test('uses the fallback without asking native code on Android', () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      var nativeCalls = 0;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_qrScannerPlatformChannel, (call) async {
            nativeCalls += 1;
            return true;
          });

      expect(await usesDynamicIslandQrScannerPortal(), isFalse);
      expect(nativeCalls, 0);
    });
  });

  testWidgets('fallback reveals the camera behind the current app surface', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(375, 667));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    var scannerClosed = false;
    await tester.pumpWidget(
      MaterialApp(
        home: FallbackPairingQrScanner(
          appSurface: const ColoredBox(
            color: Colors.white,
            child: Center(child: Text('Current app surface')),
          ),
          onClosed: (_) {
            scannerClosed = true;
          },
        ),
      ),
    );

    final sheet = find.byKey(const ValueKey('fallback-qr-scanner-app-sheet'));
    expect(tester.getRect(sheet).top, 0);
    expect(find.text('Current app surface'), findsOneWidget);

    await tester.pump(const Duration(milliseconds: 420));
    expect(tester.getRect(sheet).top, closeTo(555, 0.1));

    await tester.tapAt(const Offset(187.5, 100));
    await tester.pump();
    expect(scannerClosed, isFalse);

    await tester.pump(const Duration(milliseconds: 320));
    await tester.pumpAndSettle();
    expect(scannerClosed, isTrue);
    expect(tester.getRect(sheet).top, 0);
  });

  testWidgets(
    'portal route opens from the island and an outside tap reverses it closed',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_qrScannerPlatformChannel, (call) async {
            return switch (call.method) {
              'usesDynamicIslandQrScannerPortal' => true,
              'setDynamicIslandScannerStatusBarHidden' => null,
              _ => null,
            };
          });
      await tester.binding.setSurfaceSize(const Size(393, 852));
      addTearDown(() => tester.binding.setSurfaceSize(null));

      var scannerClosed = false;
      await tester.pumpWidget(
        MaterialApp(
          home: MediaQuery(
            data: const MediaQueryData(
              size: Size(393, 852),
              viewPadding: EdgeInsets.only(top: 59),
            ),
            child: Builder(
              builder: (context) => TextButton(
                onPressed: () async {
                  await showDynamicIslandPairingQrScanner(context);
                  scannerClosed = true;
                },
                child: const Text('Open scanner'),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Open scanner'));
      await tester.pump();

      final portal = find.byKey(
        const ValueKey('dynamic-island-qr-scanner-portal'),
      );
      expect(tester.getRect(portal), const Rect.fromLTWH(136.5, 11, 120, 36));

      await tester.pump(const Duration(milliseconds: 460));
      expect(tester.getRect(portal).top, 11);
      expect(
        find.byKey(const ValueKey('dynamic-island-qr-scanner-close')),
        findsNothing,
      );

      await tester.tapAt(const Offset(196.5, 700));
      await tester.pump();
      expect(scannerClosed, isFalse);

      await tester.pump(const Duration(milliseconds: 340));
      await tester.pumpAndSettle();
      expect(scannerClosed, isTrue);
      expect(portal, findsNothing);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('tapping the scanner closes it without a separate control', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_qrScannerPlatformChannel, (call) async {
          return switch (call.method) {
            'usesDynamicIslandQrScannerPortal' => true,
            'setDynamicIslandScannerStatusBarHidden' => null,
            _ => null,
          };
        });
    await tester.binding.setSurfaceSize(const Size(393, 852));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    var scannerClosed = false;
    await tester.pumpWidget(
      MaterialApp(
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(context).copyWith(disableAnimations: true),
          child: child!,
        ),
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () async {
              await showDynamicIslandPairingQrScanner(context);
              scannerClosed = true;
            },
            child: const Text('Open scanner'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open scanner'));
    await tester.pump();

    final portal = find.byKey(
      const ValueKey('dynamic-island-qr-scanner-portal'),
    );
    expect(tester.getRect(portal), const Rect.fromLTWH(15, 11, 363, 363));

    await tester.tapAt(tester.getCenter(portal));
    await tester.pump();
    await tester.pumpAndSettle();

    expect(scannerClosed, isTrue);
    expect(portal, findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });
}
