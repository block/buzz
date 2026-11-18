import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'skeleton.dart';

/// Fills the attachment's reserved geometry until its first frame is ready.
class MediaLoadingPlaceholder extends StatelessWidget {
  final String label;

  const MediaLoadingPlaceholder({required this.label, super.key});

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: label,
      child: LayoutBuilder(
        builder: (context, constraints) => SkeletonShimmer(
          child: SkeletonBar(
            width: constraints.hasBoundedWidth ? constraints.maxWidth : 240,
            height: constraints.hasBoundedHeight ? constraints.maxHeight : 180,
            borderRadius: BorderRadius.circular(Radii.md),
          ),
        ),
      ),
    );
  }
}
