part of '../media_viewer_page.dart';

class _MediaViewerRouteTransition extends StatelessWidget {
  final Animation<double> animation;
  final Widget child;

  const _MediaViewerRouteTransition({
    required this.animation,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final fade = CurvedAnimation(
      parent: animation,
      curve: Curves.easeOutCubic,
      reverseCurve: Curves.easeInOutCubic,
    );

    return FadeTransition(opacity: fade, child: child);
  }
}
