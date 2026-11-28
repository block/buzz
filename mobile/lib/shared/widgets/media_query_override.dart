import 'package:flutter/widgets.dart';

/// Derives [child]'s [MediaQuery] in a leaf, so the caller's build does not
/// depend on every field (keyboard insets change on each animation frame).
class MediaQueryOverride extends StatelessWidget {
  const MediaQueryOverride({
    required this.transform,
    required this.child,
    super.key,
  });

  final MediaQueryData Function(MediaQueryData) transform;
  final Widget child;

  @override
  Widget build(BuildContext context) =>
      MediaQuery(data: transform(MediaQuery.of(context)), child: child);
}
