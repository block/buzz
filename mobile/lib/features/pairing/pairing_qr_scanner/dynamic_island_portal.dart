part of '../pairing_qr_scanner.dart';

const _dynamicIslandOpenDuration = Duration(milliseconds: 460);
const _dynamicIslandCloseDuration = Duration(milliseconds: 340);
const _dynamicIslandEaseOut = Cubic(0.16, 1, 0.3, 1);

class _DynamicIslandQrScannerPortal extends HookWidget {
  const _DynamicIslandQrScannerPortal();

  @override
  Widget build(BuildContext context) {
    final controller = useMemoized(MobileScannerController.new);
    final animation = useAnimationController(
      duration: _dynamicIslandOpenDuration,
      reverseDuration: _dynamicIslandCloseDuration,
    );
    final isClosing = useState(false);
    final canPop = useState(false);
    final hasHandledResult = useRef(false);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);

    useEffect(() {
      unawaited(_setDynamicIslandScannerStatusBarHidden(true));
      if (reduceMotion) {
        animation.value = 1;
      } else {
        unawaited(animation.forward());
      }

      return () {
        unawaited(_setDynamicIslandScannerStatusBarHidden(false));
        unawaited(controller.dispose());
      };
    }, [animation, controller, reduceMotion]);

    Future<void> finish(String? result) async {
      canPop.value = true;
      await WidgetsBinding.instance.endOfFrame;
      if (context.mounted) {
        Navigator.of(context).pop(result);
      }
    }

    Future<void> closePortal() async {
      if (isClosing.value || hasHandledResult.value) {
        return;
      }
      hasHandledResult.value = true;
      isClosing.value = true;
      canPop.value = true;

      if (reduceMotion) {
        animation.value = 0;
        await WidgetsBinding.instance.endOfFrame;
      } else {
        await animation.reverse();
      }
      if (context.mounted) {
        Navigator.of(context).pop();
      }
    }

    void handleDetection(BarcodeCapture capture) {
      if (isClosing.value || hasHandledResult.value) {
        return;
      }
      final value = _firstScannedValue(capture);
      if (value == null) {
        return;
      }

      hasHandledResult.value = true;
      unawaited(_performDynamicIslandQrScanSuccessHaptic());
      unawaited(finish(value));
    }

    return PopScope(
      canPop: canPop.value,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) {
          unawaited(closePortal());
        }
      },
      child: Material(
        type: MaterialType.transparency,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: closePortal,
          child: LayoutBuilder(
            builder: (context, constraints) {
              final geometry = DynamicIslandQrScannerGeometry(
                viewport: constraints.biggest,
                safeAreaTop: MediaQuery.viewPaddingOf(context).top,
              );

              return Stack(
                children: [
                  Positioned.fill(
                    child: AnimatedBuilder(
                      animation: animation,
                      builder: (context, _) => ColoredBox(
                        color: Colors.black.withValues(
                          alpha: 0.12 * animation.value,
                        ),
                      ),
                    ),
                  ),
                  AnimatedBuilder(
                    animation: animation,
                    builder: (context, _) {
                      final progress = _dynamicIslandEaseOut.transform(
                        animation.value.clamp(0.0, 1.0),
                      );
                      final frame = geometry.frameAt(progress);
                      final scannerOpacity = geometry.scannerOpacityAt(
                        progress,
                      );
                      final introLabelOpacity = isClosing.value
                          ? 0.0
                          : geometry.introLabelOpacityAt(progress);
                      final promptOpacity = isClosing.value
                          ? 0.0
                          : math.max(introLabelOpacity, scannerOpacity);

                      return Positioned.fromRect(
                        rect: frame,
                        child: ClipRRect(
                          key: const ValueKey(
                            'dynamic-island-qr-scanner-portal',
                          ),
                          borderRadius: BorderRadius.circular(
                            geometry.cornerRadiusAt(progress),
                          ),
                          child: ColoredBox(
                            color: Colors.black,
                            child: Stack(
                              fit: StackFit.expand,
                              children: [
                                if (!isClosing.value && scannerOpacity > 0)
                                  Opacity(
                                    opacity: scannerOpacity,
                                    child: _QrScannerCamera(
                                      controller: controller,
                                      onDetect: handleDetection,
                                    ),
                                  ),
                                IgnorePointer(
                                  child: Opacity(
                                    opacity: promptOpacity,
                                    child: Center(
                                      child: Text(
                                        'Scan a QR code',
                                        style: context.textTheme.bodyMedium
                                            ?.copyWith(
                                              color: Colors.white,
                                              fontWeight: FontWeight.w600,
                                            ),
                                      ),
                                    ),
                                  ),
                                ),
                              ],
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
