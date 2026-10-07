import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/skeleton.dart';

/// Shared carousel viewport height for real media and loading placeholders.
const messageMediaCarouselHeight = 220.0;

/// Shared label, spacing, and viewport layout for an image gallery.
class MessageGalleryFrame extends StatelessWidget {
  final int count;
  final Widget child;
  final bool loading;

  const MessageGalleryFrame({
    required this.count,
    required this.child,
    this.loading = false,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final label = Text(
      '$count images',
      key: ValueKey(
        loading
            ? 'message-skeleton-carousel-count'
            : 'message-media-carousel-count',
      ),
      style: context.textTheme.labelMedium?.copyWith(
        color: context.colors.onSurfaceVariant,
        fontWeight: FontWeight.w400,
      ),
    );
    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (loading)
            ExcludeSemantics(
              child: Stack(
                alignment: Alignment.centerLeft,
                children: [
                  Opacity(opacity: 0, child: label),
                  const SkeletonBar(width: 48, height: 10),
                ],
              ),
            )
          else
            label,
          const SizedBox(height: Grid.half + Grid.quarter),
          child,
        ],
      ),
    );
  }
}
