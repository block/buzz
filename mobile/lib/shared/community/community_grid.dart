import 'package:flutter/material.dart';
import '../theme/theme.dart';

const communityGridAvatarSize = 88.0;

/// Intrinsic-height rows keep two columns without clipping scaled labels.
class CommunityGrid extends StatelessWidget {
  const CommunityGrid({super.key, required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Column(
    key: const Key('community-switcher-options'),
    mainAxisSize: MainAxisSize.min,
    children: [
      for (var index = 0; index < children.length; index += 2)
        Padding(
          padding: const EdgeInsets.only(bottom: Grid.md),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: children[index]),
              const SizedBox(width: Grid.xs),
              Expanded(
                child: index + 1 < children.length
                    ? children[index + 1]
                    : const SizedBox.shrink(),
              ),
            ],
          ),
        ),
    ],
  );
}
