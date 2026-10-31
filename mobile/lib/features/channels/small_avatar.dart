import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/agent_avatar_border.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/profile/user_profile.dart';

/// 20px avatar used in thread summary rows and other compact lists.
class SmallAvatar extends StatelessWidget {
  final String pubkey;
  final Map<String, UserProfile> userCache;
  final double size;

  /// Draws the agent shape when the caller knows this is an agent, even if
  /// its profile names no owner.
  final bool agent;

  const SmallAvatar({
    super.key,
    required this.pubkey,
    required this.userCache,
    this.size = 20,
    this.agent = false,
  });

  @override
  Widget build(BuildContext context) {
    final profile = userCache[pubkey.toLowerCase()];
    final avatarUrl = profile?.avatarUrl;
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final isAgent = agent || profile?.ownerPubkey != null;
    final side = BorderSide(color: context.colors.surface, width: 1.5);

    return Container(
      width: size,
      height: size,
      decoration: ShapeDecoration(
        shape: isAgent
            ? AgentAvatarBorder(side: side)
            : CircleBorder(side: side),
      ),
      child: AvatarImage(
        imageUrl: avatarUrl,
        radius: (size - 2) / 2,
        backgroundColor: context.controlSurface,
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
  }
}
