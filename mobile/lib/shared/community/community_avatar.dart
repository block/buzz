import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../theme/theme.dart';
import '../widgets/avatar_image.dart';
import 'community_icon_provider.dart';

/// The community icon shared by navigation and community loading surfaces.
class CommunityAvatar extends ConsumerWidget {
  final String? name;
  final String? relayUrl;
  final double size;

  const CommunityAvatar({
    super.key,
    required this.name,
    this.relayUrl,
    this.size = 40,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final trimmedName = name?.trim();
    final initial = trimmedName != null && trimmedName.isNotEmpty
        ? trimmedName.characters.first.toUpperCase()
        : '?';
    final relay = relayUrl;
    final iconUrl = relay == null
        ? null
        : ref.watch(communityIconPresentationProvider(relay));

    return ClipRSuperellipse(
      borderRadius: BorderRadius.circular(Radii.card * size / 40),
      child: ColoredBox(
        color: context.colors.primaryContainer,
        child: SizedBox.square(
          dimension: size,
          child: AvatarImageContent(
            imageUrl: iconUrl,
            fallback: Text(
              initial,
              textScaler: TextScaler.noScaling,
              style: context.textTheme.labelMedium?.copyWith(
                fontSize: size * 0.38,
                color: context.colors.onPrimaryContainer,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
