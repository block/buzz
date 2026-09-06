import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/agent_avatar_squircle.dart';
import '../../shared/profile/user_profile.dart';

/// 20px avatar used in thread summary rows and other compact lists.
class SmallAvatar extends StatelessWidget {
  final String pubkey;
  final Map<String, UserProfile> userCache;
  final double size;

  const SmallAvatar({
    super.key,
    required this.pubkey,
    required this.userCache,
    this.size = 20,
  });

  @override
  Widget build(BuildContext context) {
    final profile = userCache[pubkey.toLowerCase()];
    final avatarUrl = profile?.avatarUrl;
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final isAgent = profile?.ownerPubkey != null;

    final avatar = DecoratedBox(
      decoration: BoxDecoration(
        shape: isAgent ? BoxShape.rectangle : BoxShape.circle,
        border: Border.all(color: context.colors.surface, width: 1.5),
      ),
      child: AvatarImage(
        imageUrl: avatarUrl,
        radius: (size - 2) / 2,
        backgroundColor: context.colors.primaryContainer,
        fallback: Text(
          initial,
          style: TextStyle(
            fontSize: size * 0.4,
            fontWeight: FontWeight.w600,
            color: context.colors.onPrimaryContainer,
          ),
        ),
        isAgent: isAgent,
      ),
    );
    return SizedBox.square(
      dimension: size,
      child: isAgent ? AgentAvatarSquircle(child: avatar) : avatar,
    );
  }
}
