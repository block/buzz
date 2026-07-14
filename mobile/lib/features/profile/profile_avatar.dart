import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import 'profile_provider.dart';
import 'user_profile.dart';

/// User avatar with a presence dot indicator, for use in the app bar.
class ProfileAvatar extends ConsumerWidget {
  final VoidCallback? onTap;
  final bool showPresence;

  const ProfileAvatar({super.key, this.onTap, this.showPresence = true});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profileAsync = ref.watch(profileProvider);
    final presenceAsync = ref.watch(presenceProvider);
    final presence = presenceAsync.when(
      data: (v) => v,
      loading: () => 'online',
      error: (_, _) => 'offline',
    );

    return profileAsync.when(
      loading: () => _buildPlaceholder(context),
      error: (_, _) => _buildAvatar(context, null, presence),
      data: (profile) => _buildAvatar(context, profile, presence),
    );
  }

  Widget _buildPlaceholder(BuildContext context) {
    return CircleAvatar(
      radius: 16,
      backgroundColor: context.colors.primaryContainer,
    );
  }

  Widget _buildAvatar(
    BuildContext context,
    UserProfile? profile,
    String presence,
  ) {
    return GestureDetector(
      onTap: onTap,
      child: Stack(
        children: [
          AvatarImage(
            imageUrl: profile?.avatarUrl,
            radius: 16,
            backgroundColor: context.colors.primaryContainer,
            fallback: Text(
              profile?.initial ?? '?',
              style: context.textTheme.labelMedium?.copyWith(
                color: context.colors.onPrimaryContainer,
              ),
            ),
          ),
          if (showPresence)
            Positioned(
              right: 0,
              bottom: 0,
              child: Container(
                width: 10,
                height: 10,
                decoration: BoxDecoration(
                  color: _presenceColor(context, presence),
                  shape: BoxShape.circle,
                  border: Border.all(
                    color: context.theme.scaffoldBackgroundColor,
                    width: 1.5,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }

  Color _presenceColor(BuildContext context, String presence) {
    return switch (presence) {
      'online' => context.appColors.success,
      'away' => context.appColors.warning,
      _ => context.colors.outline,
    };
  }
}
