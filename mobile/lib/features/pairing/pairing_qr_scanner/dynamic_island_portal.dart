part of '../pairing_qr_scanner.dart';

// A smooth, non-bouncy spring inspired by Apple's public motion guidance;
// these are our timings, not private Dynamic Island animation constants.
const _dynamicIslandOpenDuration = Duration(milliseconds: 230);
const _dynamicIslandCloseDuration = Duration(milliseconds: 230);

class _DynamicIslandQrScannerPortal extends HookWidget {
  const _DynamicIslandQrScannerPortal({this.onProgress});

  final ValueChanged<double>? onProgress;

  @override
  Widget build(BuildContext context) {
    final controller = useMemoized(MobileScannerController.new);
    final animation = useAnimationController();
    final isClosing = useState(false);
    final canPop = useState(false);
    final hasHandledResult = useRef(false);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);

    useEffect(() {
      unawaited(_setDynamicIslandScannerStatusBarHidden(true));
      return () {
        unawaited(_setDynamicIslandScannerStatusBarHidden(false));
        unawaited(controller.dispose());
      };
    }, [controller]);

    useEffect(() {
      var active = true;
      void publish() {
        if (active) onProgress?.call(animation.value.clamp(0.0, 1.0));
      }

      void changed() {
        // Reduced motion can update the controller while this route builds.
        if (WidgetsBinding.instance.schedulerPhase ==
            SchedulerPhase.persistentCallbacks) {
          WidgetsBinding.instance.addPostFrameCallback((_) => publish());
        } else {
          publish();
        }
      }

      animation.addListener(changed);
      return () {
        active = false;
        animation.removeListener(changed);
      };
    }, [animation, onProgress]);

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
      unawaited(_setDynamicIslandScannerStatusBarHidden(false));
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
                                // Start warming the camera on the first frame,
                                // and keep it mounted throughout dismissal.
                                Opacity(
                                  opacity: scannerOpacity,
                                  child: _QrScannerCamera(
                                    controller: controller,
                                    onDetect: handleDetection,
                                  ),
                                ),
                                IgnorePointer(
                                  child: Opacity(
                                    opacity: scannerOpacity,
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
