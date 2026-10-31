part of '../pairing_page.dart';

// Colours and bee silhouette from https://buzz.xyz (2026-10-31).
// The bee layer uses the website’s flight model and remains decorative.
class _WebsiteWelcomeBackground extends HookWidget {
  final Widget child;

  const _WebsiteWelcomeBackground({required this.child});

  @override
  Widget build(BuildContext context) {
    final pointer = useRef<Offset?>(null);
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
              Theme(data: AppTheme.light(), child: child),
            ],
          ),
        ),
      ),
    );
  }
}
