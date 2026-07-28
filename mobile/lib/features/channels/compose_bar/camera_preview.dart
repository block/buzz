part of '../compose_bar.dart';

class _InlineCameraPreview extends HookConsumerWidget {
  final Future<void> Function(XFile image) onCapture;
  final VoidCallback onClose;

  const _InlineCameraPreview({required this.onCapture, required this.onClose});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useState<camera.CameraController?>(null);
    final controllerRef = useRef<camera.CameraController?>(null);
    final isInitializing = useState(true);
    final isCapturing = useState(false);
    final error = useState<String?>(null);

    useEffect(() {
      var disposed = false;
      var generation = 0;

      Future<void> disposeCurrent() async {
        generation += 1;
        final current = controllerRef.value;
        controllerRef.value = null;
        if (!disposed) controller.value = null;
        await current?.dispose();
      }

      Future<void> initialize() async {
        final currentGeneration = ++generation;
        if (!disposed) {
          isInitializing.value = true;
          error.value = null;
        }

        camera.CameraController? next;
        try {
          final available = await camera.availableCameras();
          if (disposed || currentGeneration != generation) return;
          if (available.isEmpty) {
            throw camera.CameraException(
              'no-cameras',
              'No cameras are available on this device.',
            );
          }

          final description = available.firstWhere(
            (candidate) =>
                candidate.lensDirection == camera.CameraLensDirection.back,
            orElse: () => available.first,
          );
          next = camera.CameraController(
            description,
            camera.ResolutionPreset.high,
            enableAudio: false,
          );
          await next.initialize();

          if (disposed || currentGeneration != generation) {
            await next.dispose();
            return;
          }
          controllerRef.value = next;
          controller.value = next;
          isInitializing.value = false;
        } catch (cameraError) {
          await next?.dispose();
          if (!disposed && currentGeneration == generation) {
            error.value = _cameraErrorMessage(cameraError);
            isInitializing.value = false;
          }
        }
      }

      final lifecycleListener = AppLifecycleListener(
        onInactive: () => unawaited(disposeCurrent()),
        onResume: () => unawaited(initialize()),
      );
      unawaited(initialize());

      return () {
        disposed = true;
        lifecycleListener.dispose();
        generation += 1;
        final current = controllerRef.value;
        controllerRef.value = null;
        unawaited(current?.dispose() ?? Future<void>.value());
      };
    }, const []);

    Future<void> capture() async {
      final activeController = controller.value;
      if (activeController == null ||
          isCapturing.value ||
          activeController.value.isTakingPicture) {
        return;
      }

      isCapturing.value = true;
      error.value = null;
      try {
        final image = await activeController.takePicture();
        if (context.mounted) {
          await processCapturedImage(image, onCapture);
        } else {
          await processCapturedImage(image, (_) async {});
        }
      } catch (captureError) {
        if (context.mounted) {
          error.value = _cameraErrorMessage(captureError);
        }
      } finally {
        if (context.mounted) isCapturing.value = false;
      }
    }

    final activeController = controller.value;
    final usesAndroidCameraLayout =
        defaultTargetPlatform == TargetPlatform.android;
    return ColoredBox(
      color: Colors.black,
      child: Stack(
        fit: StackFit.expand,
        children: [
          if (activeController case final initialized?)
            _CameraFeed(controller: initialized)
          else
            _CameraPlaceholder(
              isInitializing: isInitializing.value,
              message: error.value,
            ),
          if (activeController != null)
            Align(
              alignment: Alignment.bottomCenter,
              child: Padding(
                padding: const EdgeInsets.all(Grid.twelve),
                child: _CameraCaptureButton(
                  isPressed: isCapturing.value,
                  onTap: capture,
                ),
              ),
            ),
          Positioned(
            top: usesAndroidCameraLayout ? null : Grid.xxs,
            left: usesAndroidCameraLayout ? Grid.twelve : null,
            right: usesAndroidCameraLayout ? null : Grid.xxs,
            bottom: usesAndroidCameraLayout
                ? Grid.twelve + ((_cameraCaptureSize - _cameraBackSize) / 2)
                : null,
            child: _CameraCloseButton(
              onTap: onClose,
              emphasized: usesAndroidCameraLayout,
            ),
          ),
        ],
      ),
    );
  }
}

class _CameraFeed extends StatelessWidget {
  final camera.CameraController controller;

  const _CameraFeed({required this.controller});

  @override
  Widget build(BuildContext context) {
    final previewSize = controller.value.previewSize;
    if (previewSize == null) return const ColoredBox(color: Colors.black);

    return ClipRect(
      child: FittedBox(
        fit: BoxFit.cover,
        child: SizedBox(
          width: previewSize.height,
          height: previewSize.width,
          child: camera.CameraPreview(controller),
        ),
      ),
    );
  }
}

class _CameraPlaceholder extends StatelessWidget {
  final bool isInitializing;
  final String? message;

  const _CameraPlaceholder({
    required this.isInitializing,
    required this.message,
  });

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: Colors.black,
      child: Center(
        child: Padding(
          padding: const EdgeInsets.all(Grid.sm),
          child: isInitializing
              ? const BuzzLoadingIndicator(
                  size: 44,
                  color: Colors.white,
                  semanticLabel: 'Starting camera',
                )
              : Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Icon(
                      LucideIcons.cameraOff,
                      color: Colors.white,
                      size: 28,
                    ),
                    const SizedBox(height: Grid.xxs),
                    Text(
                      message ?? 'Camera isn’t available here.',
                      textAlign: TextAlign.center,
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: Colors.white,
                      ),
                    ),
                  ],
                ),
        ),
      ),
    );
  }
}

class _CameraCaptureButton extends StatelessWidget {
  final bool isPressed;
  final VoidCallback onTap;

  const _CameraCaptureButton({required this.isPressed, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 100);

    return Semantics(
      button: true,
      label: 'Take photo',
      child: GestureDetector(
        onTap: isPressed ? null : onTap,
        child: AnimatedScale(
          scale: isPressed ? 0.92 : 1,
          duration: duration,
          curve: Curves.easeOutCubic,
          child: Container(
            width: _cameraCaptureSize,
            height: _cameraCaptureSize,
            decoration: BoxDecoration(
              color: Colors.white.withValues(alpha: 0.24),
              shape: BoxShape.circle,
              border: Border.all(color: Colors.white, width: 3),
            ),
            padding: const EdgeInsets.all(Grid.half),
            child: const DecoratedBox(
              decoration: BoxDecoration(
                color: Colors.white,
                shape: BoxShape.circle,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _CameraCloseButton extends StatelessWidget {
  final VoidCallback onTap;
  final bool emphasized;

  const _CameraCloseButton({required this.onTap, required this.emphasized});

  @override
  Widget build(BuildContext context) {
    return SizedBox.square(
      dimension: emphasized ? _cameraBackSize : 36,
      child: IconButton(
        onPressed: onTap,
        tooltip: 'Back to attachment options',
        padding: EdgeInsets.zero,
        style: IconButton.styleFrom(
          backgroundColor: Colors.black.withValues(
            alpha: emphasized ? 0.68 : 0.56,
          ),
          foregroundColor: Colors.white,
        ),
        icon: Icon(LucideIcons.arrowLeft, size: emphasized ? 24 : 18),
      ),
    );
  }
}

const _cameraCaptureSize = 64.0;
const _cameraBackSize = 44.0;

String _cameraErrorMessage(Object error) {
  if (error is camera.CameraException) {
    return switch (error.code) {
      'CameraAccessDenied' ||
      'CameraAccessDeniedWithoutPrompt' ||
      'CameraAccessRestricted' => 'Camera access is turned off for Buzz.',
      'no-cameras' => 'Camera isn’t available on this device.',
      _ => 'Camera couldn’t start. Try again.',
    };
  }
  return 'Camera couldn’t start. Try again.';
}
