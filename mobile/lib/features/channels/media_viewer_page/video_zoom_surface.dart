part of '../media_viewer_page.dart';

class _VideoZoomSurface extends HookWidget {
  const _VideoZoomSurface({
    super.key,
    required this.child,
    required this.onTap,
    required this.controlsVisible,
    required this.onInteractionStart,
    required this.onInteractionEnd,
    required this.onDismissStart,
    required this.onDismissUpdate,
    required this.onDismissEnd,
    required this.onDismissCancel,
  });

  final Widget child;
  final VoidCallback onTap;
  final bool controlsVisible;
  final VoidCallback onInteractionStart;
  final VoidCallback onInteractionEnd;
  final VoidCallback onDismissStart;
  final ValueChanged<double> onDismissUpdate;
  final ValueChanged<double> onDismissEnd;
  final VoidCallback onDismissCancel;

  @override
  Widget build(BuildContext context) {
    final transform = useTransformationController();
    final transformed = useState(false);
    final dismissGesture = useRef(false);
    final resetStart = useRef(Matrix4.identity());
    final gestureGeneration = useState(0);
    final resetRequest = useRef(0);
    final reset = useAnimationController(
      duration: const Duration(milliseconds: 180),
    );

    useEffect(() {
      void updateTransform() {
        transformed.value = _hasImageTransform(transform.value);
        if (transformed.value && dismissGesture.value) {
          dismissGesture.value = false;
          onDismissCancel();
        }
      }

      void updateReset() {
        transform.value = Matrix4Tween(
          begin: resetStart.value,
          end: Matrix4.identity(),
        ).transform(Curves.easeOutCubic.transform(reset.value));
      }

      transform.addListener(updateTransform);
      reset.addListener(updateReset);
      return () {
        transform.removeListener(updateTransform);
        reset.removeListener(updateReset);
      };
    }, [transform, reset, onDismissCancel]);

    void resetToFit({bool cancelMomentum = false}) {
      final request = ++resetRequest.value;
      reset.stop();
      dismissGesture.value = false;
      onDismissCancel();
      resetStart.value = Matrix4.copy(transform.value);
      void startReset() {
        if (!context.mounted || resetRequest.value != request) return;
        if (MediaQuery.disableAnimationsOf(context)) {
          transform.value = Matrix4.identity();
        } else {
          reset.forward(from: 0);
        }
      }

      if (cancelMomentum) {
        // Dispose the viewer's internal fling animation so it cannot keep
        // moving the shared transform after the double-tap reset.
        gestureGeneration.value++;
        // Wait for that old viewer to dispose before writing the reset,
        // including when reduced motion makes the reset instantaneous.
        WidgetsBinding.instance.addPostFrameCallback((_) => startReset());
      } else {
        startReset();
      }
    }

    return Semantics(
      button: true,
      label: controlsVisible ? 'Hide video controls' : 'Show video controls',
      onTap: onTap,
      child: GestureDetector(
        excludeFromSemantics: true,
        key: const ValueKey('message-media-video-viewer-gesture'),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        onDoubleTap: transformed.value
            ? () {
                onInteractionEnd();
                resetToFit(cancelMomentum: true);
              }
            : null,
        child: KeyedSubtree(
          key: ValueKey(gestureGeneration.value),
          child: InteractiveViewer(
            key: const ValueKey('message-media-video-viewer-zoom'),
            transformationController: transform,
            minScale: 1,
            maxScale: 4,
            panEnabled: transformed.value,
            boundaryMargin: const EdgeInsets.all(Grid.xxl),
            clipBehavior: Clip.none,
            onInteractionStart: (details) {
              resetRequest.value++;
              reset.stop();
              onInteractionStart();
              dismissGesture.value =
                  details.pointerCount == 1 && !transformed.value;
              if (dismissGesture.value) onDismissStart();
            },
            onInteractionUpdate: (details) {
              if (details.pointerCount > 1 || details.scale != 1) {
                if (dismissGesture.value) onDismissCancel();
                dismissGesture.value = false;
              } else if (dismissGesture.value && !transformed.value) {
                onDismissUpdate(details.focalPointDelta.dy);
              }
            },
            onInteractionEnd: (details) {
              onInteractionEnd();
              if (dismissGesture.value) {
                dismissGesture.value = false;
                onDismissEnd(details.velocity.pixelsPerSecond.dy);
              } else if (transform.value.getMaxScaleOnAxis() <= 1.001) {
                // Finish a pinch back to fit at the original centered position,
                // so the next downward swipe can dismiss rather than pan.
                resetToFit();
              }
            },
            child: child,
          ),
        ),
      ),
    );
  }
}
