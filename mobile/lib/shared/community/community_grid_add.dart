import 'package:flutter/material.dart';
import '../theme/theme.dart';
import '../theme/buzz_icons.dart';
import 'community_grid.dart';

/// Opens the Add Community flow from a community grid.
class CommunityGridAdd extends StatelessWidget {
  const CommunityGridAdd({super.key, required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    child: InkWell(
      key: const Key('community-switcher-add'),
      borderRadius: BorderRadius.circular(Radii.card),
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        child: Column(
          children: [
            ClipRSuperellipse(
              borderRadius: BorderRadius.circular(
                Radii.card * communityGridAvatarSize / 40,
              ),
              child: Container(
                width: communityGridAvatarSize,
                height: communityGridAvatarSize,
                color: context.controlSurface.withValues(alpha: 0.5),
                child: Icon(
                  BuzzIcons.plus,
                  size: 32,
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
            const SizedBox(height: Grid.xs),
            Text(
              'Add Community',
              textAlign: TextAlign.center,
              style: context.textTheme.bodyLarge?.copyWith(
                fontWeight: FontWeight.w500,
              ),
            ),
          ],
        ),
      ),
    ),
  );
}
