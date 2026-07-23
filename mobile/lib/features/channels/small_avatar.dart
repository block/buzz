import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../profile/user_profile.dart';

/// 20px circle avatar used in thread summary rows and other compact lists.
class SmallAvatar extends StatelessWidget {
  final String pubkey;
  final Map<String, UserProfile> userCache;

  const SmallAvatar({super.key, required this.pubkey, required this.userCache});

  @override
  Widget build(BuildContext context) {
    final profile = userCache[pubkey.toLowerCase()];
    final avatarUrl = profile?.avatarUrl;
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');

    return Container(
      width: 20,
      height: 20,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        border: Border.all(color: context.colors.surface, width: 1.5),
      ),
      child: AvatarImage(
        imageUrl: avatarUrl,
        radius: 9,
        backgroundColor: context.colors.primaryContainer,
        fallback: Text(
          initial,
          style: TextStyle(
            fontSize: 8,
            fontWeight: FontWeight.w600,
            color: context.colors.onPrimaryContainer,
          ),
        ),
      ),
    );
  }
}
