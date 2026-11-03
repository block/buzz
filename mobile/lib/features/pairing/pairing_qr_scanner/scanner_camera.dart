part of '../pairing_qr_scanner.dart';

class _QrScannerCamera extends HookWidget {
  const _QrScannerCamera({required this.controller, required this.onDetect});

  final MobileScannerController controller;
  final ValueChanged<BarcodeCapture> onDetect;

  @override
  Widget build(BuildContext context) {
    final cameraState = useState(controller.value);
    useEffect(() {
      var active = true;
      void publish() {
        if (active) cameraState.value = controller.value;
      }

      void changed() {
        // The scanner can publish initialization state while its child builds.
        if (WidgetsBinding.instance.schedulerPhase ==
            SchedulerPhase.persistentCallbacks) {
          WidgetsBinding.instance.addPostFrameCallback((_) => publish());
        } else {
          publish();
        }
      }

      controller.addListener(changed);
      return () {
        active = false;
        controller.removeListener(changed);
      };
    }, [controller]);
    final state = cameraState.value;
    final ready = state.isInitialized && state.isRunning;
    return AnimatedOpacity(
      key: const ValueKey('qr-camera-ready-fade'),
      opacity: ready || state.error != null ? 1 : 0,
      duration: MediaQuery.disableAnimationsOf(context) || state.error != null
          ? Duration.zero
          : const Duration(milliseconds: 230),
      curve: Curves.easeOut,
      child: MobileScanner(
        controller: controller,
        fit: BoxFit.cover,
        errorBuilder: (context, error) {
          final message = switch (error.errorCode) {
            MobileScannerErrorCode.permissionDenied =>
              'Camera permission is required to scan QR codes.\n\n'
                  'Please grant camera access in your device settings.',
            _ =>
              'Could not start camera: '
                  "${error.errorDetails?.message ?? 'unknown error'}",
          };
          return ColoredBox(
            color: Colors.black,
            child: Center(
              child: Padding(
                padding: const EdgeInsets.all(Grid.sm),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(
                      BuzzIcons.cameraOff,
                      size: 48,
                      color: Colors.white.withValues(alpha: 0.72),
                    ),
                    const SizedBox(height: Grid.xs),
                    Text(
                      message,
                      textAlign: TextAlign.center,
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: Colors.white.withValues(alpha: 0.72),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          );
        },
        onDetect: onDetect,
      ),
    );
  }
}
