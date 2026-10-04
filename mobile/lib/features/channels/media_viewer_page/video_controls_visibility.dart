part of '../media_viewer_page.dart';

/// Only playback transitions affect visibility; position ticks must never
/// restart the idle timer or force the entire video route to rebuild.
({
  bool visible,
  VoidCallback toggle,
  VoidCallback beginInteraction,
  VoidCallback endInteraction,
})
_useVideoControlsVisibility(
  BuildContext context,
  VideoPlayerController? controller,
) {
  final visible = useState(true);
  final playing = useState(false);
  final interacting = useState(false);
  final interactionVersion = useState(0);
  final lifecycle = useAppLifecycleState();
  final accessibleNavigation = MediaQuery.accessibleNavigationOf(context);
  final isCurrentRoute = ModalRoute.of(context)?.isCurrent ?? true;

  useEffect(() {
    void updatePlayback() {
      final value = controller?.value;
      final active =
          value != null &&
          value.isInitialized &&
          value.isPlaying &&
          !value.isBuffering &&
          !value.hasError;
      if (playing.value == active) return;
      playing.value = active;
      if (!active) visible.value = true;
    }

    updatePlayback();
    controller?.addListener(updatePlayback);
    return () => controller?.removeListener(updatePlayback);
  }, [controller]);

  useEffect(
    () {
      if (!visible.value ||
          !playing.value ||
          interacting.value ||
          accessibleNavigation ||
          !isCurrentRoute ||
          (lifecycle != null && lifecycle != AppLifecycleState.resumed)) {
        return null;
      }
      final timer = Timer(
        const Duration(seconds: 3),
        () => visible.value = false,
      );
      return timer.cancel;
    },
    [
      visible.value,
      playing.value,
      interacting.value,
      interactionVersion.value,
      accessibleNavigation,
      isCurrentRoute,
      lifecycle,
    ],
  );

  return (
    visible: visible.value,
    toggle: () => visible.value = !visible.value,
    beginInteraction: () {
      interacting.value = true;
      visible.value = true;
    },
    endInteraction: () {
      interacting.value = false;
      interactionVersion.value++;
    },
  );
}

class _VideoViewerChrome extends StatelessWidget {
  const _VideoViewerChrome({
    super.key,
    required this.visible,
    required this.dragOpacity,
    required this.child,
  });

  final bool visible;
  final double dragOpacity;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final interactive = visible && dragOpacity > 0;
    return IgnorePointer(
      ignoring: !interactive,
      child: ExcludeSemantics(
        excluding: !interactive,
        child: AnimatedOpacity(
          opacity: visible ? dragOpacity : 0,
          duration: MediaQuery.disableAnimationsOf(context)
              ? Duration.zero
              : const Duration(milliseconds: 180),
          curve: Curves.easeOutCubic,
          child: child,
        ),
      ),
    );
  }
}
