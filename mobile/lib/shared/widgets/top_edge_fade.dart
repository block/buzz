import 'package:flutter/material.dart';

/// Softens content entering a fixed header while retaining the viewport's clip.
/// Only the first 16 logical pixels fade; controls and the bottom stay sharp.
class TopEdgeFade extends StatelessWidget {
  const TopEdgeFade({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => ClipRect(
    child: MediaQuery.highContrastOf(context)
        ? child
        : ShaderMask(
            blendMode: BlendMode.dstIn,
            shaderCallback: (bounds) =>
                const LinearGradient(
                  begin: Alignment.topCenter,
                  end: Alignment.bottomCenter,
                  colors: [Colors.transparent, Colors.white],
                ).createShader(
                  Rect.fromLTWH(bounds.left, bounds.top, bounds.width, 16),
                ),
            child: child,
          ),
  );
}
