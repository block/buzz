part of '../pairing_page.dart';

// Colours and bee silhouette from https://buzz.xyz (2026-10-31).
// The bee layer uses the website’s flight model and remains decorative.
class _WebsiteWelcomeBackground extends HookWidget {
  final Widget child;
  final ValueListenable<double> scannerProgress;

  const _WebsiteWelcomeBackground({
    required this.child,
    required this.scannerProgress,
  });

  @override
  Widget build(BuildContext context) {
    final pointer = useRef<Offset?>(null);
    final theme = AppTheme.light();
    return MouseRegion(
      onHover: (event) => pointer.value = event.localPosition,
      onExit: (_) => pointer.value = null,
      child: Listener(
        onPointerDown: (event) => pointer.value = event.localPosition,
        onPointerMove: (event) => pointer.value = event.localPosition,
        onPointerUp: (_) => pointer.value = null,
        onPointerCancel: (_) => pointer.value = null,
        child: DecoratedBox(
          key: const Key('pairing-website-welcome-background'),
          decoration: const BoxDecoration(color: Color(0xFFD7D72E)),
          child: Stack(
            fit: StackFit.expand,
            children: [
              if (MediaQuery.textScalerOf(context).scale(16) <= 24)
                Positioned.fill(
                  child: IgnorePointer(
                    child: ExcludeSemantics(
                      child: ValueListenableBuilder<double>(
                        valueListenable: scannerProgress,
                        builder: (context, progress, child) => Opacity(
                          key: const Key('scanner-bees-opacity'),
                          opacity: 1 - progress,
                          child: Transform.scale(
                            key: const Key('scanner-bees-scale'),
                            scale: MediaQuery.disableAnimationsOf(context)
                                ? 1
                                : 1 - 0.04 * progress,
                            child: TickerMode(
                              enabled: progress < 1,
                              child: child!,
                            ),
                          ),
                        ),
                        child: LayoutBuilder(
                          builder: (context, constraints) => RepaintBoundary(
                            child: _WebsiteBeeSwarm(
                              size: constraints.biggest,
                              pointer: pointer,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              Theme(
                data: theme.copyWith(
                  textButtonTheme: TextButtonThemeData(
                    style: theme.textButtonTheme.style?.copyWith(
                      backgroundColor: const WidgetStatePropertyAll(
                        Color(0xFFD7D72E),
                      ),
                    ),
                  ),
                ),
                child: child,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
