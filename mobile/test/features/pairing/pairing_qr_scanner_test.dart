import 'dart:async';

import 'package:buzz/features/pairing/pairing_qr_scanner.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

const _qrScannerPlatformChannel = MethodChannel('buzz/qr_scanner');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final defaultScannerPlatform = MobileScannerPlatform.instance;
  late _FakeMobileScannerPlatform fakeScannerPlatform;

  setUp(() {
    fakeScannerPlatform = _FakeMobileScannerPlatform();
    MobileScannerPlatform.instance = fakeScannerPlatform;
  });

  tearDown(() async {
    debugDefaultTargetPlatformOverride = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_qrScannerPlatformChannel, null);
    await fakeScannerPlatform.dispose();
    MobileScannerPlatform.instance = defaultScannerPlatform;
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
      expect(
        tester.widget<ClipRSuperellipse>(portal).borderRadius,
        BorderRadius.circular(18),
      );
      expect(
        tester.widget<ClipRSuperellipse>(portal).clipBehavior,
        Clip.antiAlias,
      );

      await tester.pump(const Duration(milliseconds: 16));
      final firstOpeningFrame = tester.getRect(portal);
      expect(firstOpeningFrame.height, inExclusiveRange(36, 50));
      await tester.pump(const Duration(milliseconds: 214));
      final openingRadius =
          (tester.widget<ClipRSuperellipse>(portal).borderRadius
                  as BorderRadius)
              .topLeft
              .x;
      expect(openingRadius, inExclusiveRange(18, 40));

      await tester.pumpAndSettle();
      expect(
        tester.widget<ClipRSuperellipse>(portal).borderRadius,
        BorderRadius.circular(40),
      );
      expect(tester.getRect(portal).top, 11);
      expect(
        find.byKey(const ValueKey('dynamic-island-qr-scanner-close')),
        findsNothing,
      );

      await tester.tapAt(const Offset(196.5, 700));
      await tester.pump();
      expect(scannerClosed, isFalse);
      expect(find.byType(MobileScanner), findsOneWidget);

      await tester.pump(const Duration(milliseconds: 100));
      final earlyClosingHeight = tester.getRect(portal).height;
      expect(earlyClosingHeight, lessThan(280));
      expect(find.byType(MobileScanner), findsOneWidget);
      await tester.pump(const Duration(milliseconds: 200));
      final lateClosingHeight = tester.getRect(portal).height;
      // The camera is transparent here, but must not shut down mid-collapse.
      expect(find.byType(MobileScanner), findsOneWidget);
      expect(
        tester
            .widget<Opacity>(
              find
                  .ancestor(
                    of: find.byType(MobileScanner),
                    matching: find.byType(Opacity),
                  )
                  .first,
            )
            .opacity,
        0,
      );
      expect(fakeScannerPlatform.stopCalls, 0);
      expect(fakeScannerPlatform._isDisposed, isFalse);
      expect(lateClosingHeight, lessThan(earlyClosingHeight));
      expect(lateClosingHeight, greaterThan(36));
      // The final approach is slower than the initial collapse, not an impact.
      expect(lateClosingHeight - 36, lessThan((363 - earlyClosingHeight) / 4));
      final closingRadius =
          (tester.widget<ClipRSuperellipse>(portal).borderRadius
                  as BorderRadius)
              .topLeft
              .x;
      expect(closingRadius, inExclusiveRange(18, 40));
      await tester.pump(const Duration(milliseconds: 170));
      await tester.pumpAndSettle();
      expect(scannerClosed, isTrue);
      expect(portal, findsNothing);
      expect(fakeScannerPlatform._isDisposed, isTrue);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets(
    'an early dismissal keeps its frame and settles back into the island',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(393, 852));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      var closeCount = 0;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) => TextButton(
              onPressed: () async {
                await showDynamicIslandPairingQrScanner(context);
                closeCount++;
              },
              child: const Text('Open scanner'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open scanner'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 80));
      final portal = find.byKey(
        const ValueKey('dynamic-island-qr-scanner-portal'),
      );
      final beforeDismissal = tester.getRect(portal);
      await tester.tapAt(const Offset(196.5, 700));
      await tester.pump();
      expect(tester.getRect(portal), beforeDismissal);
      // Preserve the outgoing velocity briefly instead of abruptly flipping it.
      await tester.pump(const Duration(milliseconds: 8));
      final afterDismissal = tester.getRect(portal);
      expect(afterDismissal.height, greaterThan(beforeDismissal.height));
      expect(afterDismissal.height - beforeDismissal.height, lessThan(12));
      await tester.tapAt(const Offset(196.5, 700));
      await tester.pumpAndSettle();
      expect(closeCount, 1);
      expect(portal, findsNothing);
      expect(find.text('Open scanner'), findsOneWidget);
      expect(tester.takeException(), isNull);
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
    expect(
      tester.widget<ClipRSuperellipse>(portal).borderRadius,
      BorderRadius.circular(40),
    );

    await tester.tapAt(tester.getCenter(portal));
    await tester.pump();
    await tester.pumpAndSettle();

    expect(scannerClosed, isTrue);
    expect(portal, findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });
}

class _FakeMobileScannerPlatform extends MobileScannerPlatform {
  final _barcodes = StreamController<BarcodeCapture?>.broadcast();
  var _isDisposed = false;
  var stopCalls = 0;

  @override
  Stream<BarcodeCapture?> get barcodesStream => _barcodes.stream;

  @override
  Stream<TorchState> get torchStateStream =>
      Stream.value(TorchState.unavailable);

  @override
  Stream<double> get zoomScaleStateStream => Stream.value(1);

  @override
  Future<MobileScannerViewAttributes> start(StartOptions startOptions) async {
    return const MobileScannerViewAttributes(
      cameraDirection: CameraFacing.back,
      currentTorchMode: TorchState.unavailable,
      size: Size(200, 200),
      numberOfCameras: 1,
    );
  }

  @override
  Widget buildCameraView() => const SizedBox.expand();

  @override
  Future<void> stop() async {
    stopCalls++;
  }

  @override
  Future<void> dispose() async {
    if (_isDisposed) {
      return;
    }
    _isDisposed = true;
    await _barcodes.close();
  }
}
