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
    return Container(
      width: double.infinity,
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: Colors.black,
        borderRadius: BorderRadius.circular(Radii.dialog),
      ),
      foregroundDecoration: BoxDecoration(
        borderRadius: BorderRadius.circular(Radii.dialog),
        border: Border.all(
          color: Colors.black.withValues(alpha: 0.04),
          width: 1,
        ),
      ),
      child: AspectRatio(
        aspectRatio: 4 / 3,
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
                    isCapturing: isCapturing.value,
                    onTap: capture,
                  ),
                ),
              ),
            Positioned(
              top: Grid.xxs,
              right: Grid.xxs,
              child: _CameraCloseButton(onTap: onClose),
            ),
          ],
        ),
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
              ? const CircularProgressIndicator(
                  color: Colors.white,
                  strokeWidth: 2,
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
  final bool isCapturing;
  final VoidCallback onTap;

  const _CameraCaptureButton({required this.isCapturing, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return Semantics(
      button: true,
      label: 'Take photo',
      child: GestureDetector(
        onTap: isCapturing ? null : onTap,
        child: Container(
          width: 64,
          height: 64,
          decoration: BoxDecoration(
            color: Colors.white.withValues(alpha: 0.24),
            shape: BoxShape.circle,
            border: Border.all(color: Colors.white, width: 3),
          ),
          padding: const EdgeInsets.all(Grid.half),
          child: DecoratedBox(
            decoration: const BoxDecoration(
              color: Colors.white,
              shape: BoxShape.circle,
            ),
            child: isCapturing
                ? const Padding(
                    padding: EdgeInsets.all(Grid.twelve),
                    child: CircularProgressIndicator(
                      color: Colors.black,
                      strokeWidth: 2,
                    ),
                  )
                : null,
          ),
        ),
      ),
    );
  }
}

class _CameraCloseButton extends StatelessWidget {
  final VoidCallback onTap;

  const _CameraCloseButton({required this.onTap});

  @override
  Widget build(BuildContext context) {
    return SizedBox.square(
      dimension: 36,
      child: IconButton(
        onPressed: onTap,
        tooltip: 'Close camera',
        padding: EdgeInsets.zero,
        style: IconButton.styleFrom(
          backgroundColor: Colors.black.withValues(alpha: 0.56),
          foregroundColor: Colors.white,
        ),
        icon: const Icon(LucideIcons.x, size: 18),
      ),
    );
  }
}

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
