import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../profile/user_profile.dart';

/// 20px circle avatar used in thread summary rows and other compact lists.
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

    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
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
      ),
    );
  }
}
