part of '../pairing_qr_scanner.dart';

const _fallbackScannerOpenDuration = Duration(milliseconds: 420);
const _fallbackScannerCloseDuration = Duration(milliseconds: 320);
const _fallbackScannerSheetPeekHeight = 112.0;
const _fallbackScannerSheetCornerRadius = 32.0;
const _fallbackScannerDrawerCurve = Cubic(0.32, 0.72, 0, 1);

/// Reveals the scanner behind the current app surface on standard devices.
///
/// The app surface moves down as a rounded sheet instead of navigating to a
/// separate scanner page. This is used by Android and iPhones without a
/// Dynamic Island.
class FallbackPairingQrScanner extends HookWidget {
  const FallbackPairingQrScanner({
    required this.appSurface,
    required this.onClosed,
    super.key,
  });

  /// The current Buzz screen that moves down to reveal the camera.
  final Widget appSurface;

  /// Called after the app surface has returned, with a scanned value if any.
  final ValueChanged<String?> onClosed;

  @override
  Widget build(BuildContext context) {
    final controller = useMemoized(MobileScannerController.new);
    final animation = useAnimationController(
      duration: _fallbackScannerOpenDuration,
      reverseDuration: _fallbackScannerCloseDuration,
    );
    final isClosing = useRef(false);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);

    useEffect(
      () => () {
        unawaited(controller.dispose());
      },
      [controller],
    );

    useEffect(() {
      if (reduceMotion) {
        animation.value = 1;
      } else {
        unawaited(animation.forward());
      }
      return null;
    }, [animation, reduceMotion]);

    Future<void> closeScanner([String? result]) async {
      if (isClosing.value) {
        return;
      }
      isClosing.value = true;

      if (reduceMotion) {
        animation.value = 0;
        await WidgetsBinding.instance.endOfFrame;
      } else {
        await animation.reverse();
      }

      if (context.mounted) {
        onClosed(result);
      }
    }

    void handleDetection(BarcodeCapture capture) {
      if (isClosing.value) {
        return;
      }
      final value = _firstScannedValue(capture);
      if (value == null) {
        return;
      }

      unawaited(closeScanner(value));
    }

    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) {
          unawaited(closeScanner());
        }
      },
      child: AnnotatedRegion<SystemUiOverlayStyle>(
        value: SystemUiOverlayStyle.light,
        child: Material(
          key: const ValueKey('fallback-qr-scanner-reveal'),
          color: Colors.black,
          child: LayoutBuilder(
            builder: (context, constraints) {
              final viewport = constraints.biggest;
              final sheetTravel = math.max(
                0.0,
                viewport.height - _fallbackScannerSheetPeekHeight,
              );

              return Stack(
                clipBehavior: Clip.hardEdge,
                children: [
                  Positioned.fill(
                    child: Semantics(
                      button: true,
                      label: 'Close QR scanner',
                      child: GestureDetector(
                        behavior: HitTestBehavior.opaque,
                        onTap: closeScanner,
                        child: Stack(
                          fit: StackFit.expand,
                          children: [
                            _QrScannerCamera(
                              controller: controller,
                              onDetect: handleDetection,
                            ),
                            const IgnorePointer(
                              child: _FallbackQrScannerViewfinder(),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                  AnimatedBuilder(
                    animation: animation,
                    child: appSurface,
                    builder: (context, child) {
                      final progress = _fallbackScannerDrawerCurve.transform(
                        animation.value.clamp(0.0, 1.0),
                      );
                      final offset = sheetTravel * progress;
                      final radius =
                          _fallbackScannerSheetCornerRadius * progress;

                      return Positioned(
                        top: offset,
                        left: 0,
                        right: 0,
                        height: viewport.height,
                        child: Semantics(
                          button: true,
                          label: 'Close QR scanner',
                          child: GestureDetector(
                            behavior: HitTestBehavior.opaque,
                            onTap: closeScanner,
                            child: ClipRRect(
                              key: const ValueKey(
                                'fallback-qr-scanner-app-sheet',
                              ),
                              borderRadius: BorderRadius.vertical(
                                top: Radius.circular(radius),
                              ),
                              child: Stack(
                                fit: StackFit.expand,
                                children: [
                                  AbsorbPointer(child: child),
                                  if (progress > 0)
                                    Align(
                                      alignment: Alignment.topCenter,
                                      child: Padding(
                                        padding: const EdgeInsets.only(
                                          top: Grid.twelve,
                                        ),
                                        child: Opacity(
                                          opacity: progress,
                                          child: Container(
                                            width: Grid.md,
                                            height: Grid.half,
                                            decoration: BoxDecoration(
                                              color: context
                                                  .colors
                                                  .onSurfaceVariant
                                                  .withValues(alpha: 0.45),
                                              borderRadius:
                                                  BorderRadius.circular(
                                                    Grid.half,
                                                  ),
                                            ),
                                          ),
                                        ),
                                      ),
                                    ),
                                ],
                              ),
                            ),
                          ),
                        ),
                      );
                    },
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}

class _FallbackQrScannerViewfinder extends StatelessWidget {
  const _FallbackQrScannerViewfinder();

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final size = math.min(
          constraints.maxWidth - (Grid.xl * 2),
          constraints.maxHeight - 240,
        );

        return Stack(
          fit: StackFit.expand,
          children: [
            CustomPaint(
              painter: _FallbackQrScannerViewfinderPainter(
                viewfinderSize: math.max(0, size),
              ),
            ),
            Align(
              alignment: Alignment.center,
              child: Transform.translate(
                offset: Offset(0, (size / 2) + Grid.md),
                child: Text(
                  'Scan a QR code',
                  style: context.textTheme.bodyMedium?.copyWith(
                    color: Colors.white,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}

class _FallbackQrScannerViewfinderPainter extends CustomPainter {
  const _FallbackQrScannerViewfinderPainter({required this.viewfinderSize});

  final double viewfinderSize;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Rect.fromCenter(
      center: size.center(Offset.zero),
      width: viewfinderSize,
      height: viewfinderSize,
    );
    final roundedRect = RRect.fromRectAndRadius(
      rect,
      const Radius.circular(40),
    );

    canvas.saveLayer(Offset.zero & size, Paint());
    canvas.drawRect(
      Offset.zero & size,
      Paint()..color = Colors.black.withValues(alpha: 0.6),
    );
    canvas.drawRRect(roundedRect, Paint()..blendMode = BlendMode.clear);
    canvas.restore();
    canvas.drawRRect(
      roundedRect,
      Paint()
        ..color = Colors.white
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3,
    );
  }

  @override
  bool shouldRepaint(_FallbackQrScannerViewfinderPainter oldDelegate) {
    return oldDelegate.viewfinderSize != viewfinderSize;
  }
}
