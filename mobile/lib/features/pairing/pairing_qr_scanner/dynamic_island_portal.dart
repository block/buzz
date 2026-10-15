part of '../pairing_qr_scanner.dart';

// A smooth, non-bouncy spring inspired by Apple's public motion guidance;
// these are our timings, not private Dynamic Island animation constants.
const _dynamicIslandOpenDuration = Duration(milliseconds: 440);
const _dynamicIslandCloseDuration = Duration(milliseconds: 360);

class _DynamicIslandQrScannerPortal extends HookWidget {
  const _DynamicIslandQrScannerPortal();

  @override
  Widget build(BuildContext context) {
    final controller = useMemoized(MobileScannerController.new);
    final animation = useAnimationController();
    final isClosing = useState(false);
    final canPop = useState(false);
    final hasHandledResult = useRef(false);
    final cameraMounted = useRef(false);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);

    useEffect(() {
      unawaited(_setDynamicIslandScannerStatusBarHidden(true));
      return () {
        unawaited(_setDynamicIslandScannerStatusBarHidden(false));
        unawaited(controller.dispose());
      };
    }, [controller]);

    Future<void> finish(String? result) async {
      if (canPop.value) return;
      canPop.value = true;
      await WidgetsBinding.instance.endOfFrame;
      if (context.mounted) {
        Navigator.of(context).pop(result);
      }
    }

    useEffect(() {
      Future<void> transition() async {
        final closing = isClosing.value;
        final target = closing ? 0.0 : 1.0;
        try {
          if (reduceMotion) {
            animation.value = target;
          } else {
            await animation
                .animateWith(
                  SpringSimulation(
                    SpringDescription.withDurationAndBounce(
                      duration: closing
                          ? _dynamicIslandCloseDuration
                          : _dynamicIslandOpenDuration,
                      bounce: 0,
                    ),
                    animation.value,
                    target,
                    animation.velocity,
                    tolerance: const Tolerance(distance: 0.001, velocity: 0.02),
                    snapToEnd: true,
                  ),
                )
                .orCancel;
          }
          if (closing && context.mounted) await finish(null);
        } on TickerCanceled {
          // A dismissal, motion-setting change, or unmount can retarget the spring.
        }
      }

      unawaited(transition());
      return null;
    }, [animation, reduceMotion, isClosing.value]);

    void closePortal() {
      if (isClosing.value || hasHandledResult.value) return;
      hasHandledResult.value = true;
      isClosing.value = true;
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
          closePortal();
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
                      final progress = animation.value.clamp(0.0, 1.0);
                      final frame = geometry.frameAt(progress);
                      final scannerOpacity = geometry.scannerOpacityAt(
                        progress,
                      );
                      // Once revealed, keep the camera mounted until the route
                      // closes. Native camera teardown can stall a moving frame.
                      if (scannerOpacity > 0) cameraMounted.value = true;
                      final introLabelOpacity = isClosing.value
                          ? 0.0
                          : geometry.introLabelOpacityAt(progress);
                      final promptOpacity = math.max(
                        introLabelOpacity,
                        scannerOpacity,
                      );

                      return Positioned.fromRect(
                        rect: frame,
                        // Continuous corners stay smooth throughout the island morph.
                        child: ClipRSuperellipse(
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
                                if (cameraMounted.value)
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
