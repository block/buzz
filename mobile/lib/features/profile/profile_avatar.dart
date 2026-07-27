import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/masked_avatar_badge.dart';
import 'profile_provider.dart';
import 'user_profile.dart';

/// Matches desktop's sidebar profile card, whose avatar is 32px.
const _avatarSize = 32.0;

/// The visible dot is smaller than the notch it sits in, so a ring of
/// background separates it from the avatar. Desktop's `h-2 w-2` dot inside a
/// `h-3.5 w-3.5` badge frame.
const _presenceDotRatio = 8 / 14;

/// User avatar with a presence dot indicator, for use in the app bar.
///
/// The dot sits in a notch masked out of the avatar rather than overlapping it,
/// the same treatment desktop's `SidebarProfileCard` uses — so it needs no
/// background-coloured ring, and reads correctly over the themed top section.
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
      radius: _avatarSize / 2,
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
      child: MaskedAvatarBadge(
        size: _avatarSize,
        geometry: AvatarBadgeMaskGeometry.presenceDot,
        avatar: ClipOval(
          child: ColoredBox(
            color: context.colors.primaryContainer,
            child: AvatarImageContent(
              imageUrl: profile?.avatarUrl,
              fallback: Text(
                profile?.initial ?? '?',
                style: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.onPrimaryContainer,
                ),
              ),
            ),
          ),
        ),
        badge: showPresence
            ? Center(
                child: FractionallySizedBox(
                  widthFactor: _presenceDotRatio,
                  heightFactor: _presenceDotRatio,
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: _presenceColor(context, presence),
                      shape: BoxShape.circle,
                    ),
                  ),
                ),
              )
            : null,
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
