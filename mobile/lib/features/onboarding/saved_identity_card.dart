import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/device_identity_source.dart';
import '../../shared/hosted/saved_identity_profile.dart';
import '../../shared/hosted/hosted_environment.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/avatar_image.dart';

/// An explicit saved-key choice using available cached profile metadata.
class SavedIdentityCard extends ConsumerWidget {
  const SavedIdentityCard({
    super.key,
    required this.identity,
    required this.onSelect,
  });

  final DeviceIdentityChoice identity;
  final VoidCallback? onSelect;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final cachedProfile = ref.watch(
      userCacheProvider.select((profiles) => profiles[identity.pubkey]),
    );
    final profile =
        cachedProfile ??
        ref.watch(savedIdentityProfileProvider(identity.pubkey)).asData?.value;
    final source = identity.label
        .split(', ')
        .map(
          (name) => HostedEnvironment.communityDisplayName(name, 'wss://$name'),
        )
        .join(', ');
    final profileName = profile?.displayName;
    final name = profileName != null && profileName.trim().isNotEmpty
        ? profileName
        : 'Saved identity';
    return Semantics(
      button: true,
      enabled: onSelect != null,
      label: 'Use $name, ${shortPubkey(identity.pubkey)}',
      onTap: onSelect,
      excludeSemantics: true,
      child: OutlinedButton(
        onPressed: onSelect,
        style: OutlinedButton.styleFrom(
          backgroundColor: context.colors.surfaceContainerHighest,
          padding: const EdgeInsets.all(Grid.xs),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.card),
          ),
          foregroundColor: context.colors.onSurface,
          alignment: Alignment.centerLeft,
        ),
        child: Row(
          children: [
            AvatarImage(
              imageUrl: profile?.avatarUrl,
              radius: 24,
              fallback: Center(
                child: Text(
                  name.isEmpty ? '?' : name.characters.first.toUpperCase(),
                ),
              ),
            ),
            const SizedBox(width: Grid.twelve),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(name, style: context.textTheme.titleMedium),
                  if (source != 'Saved identity')
                    Text('Used in $source', style: context.textTheme.bodySmall),
                  const SizedBox(height: Grid.half),
                  Text(
                    shortPubkey(identity.pubkey),
                    style: context.textTheme.bodySmall,
                  ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            const Icon(BuzzIcons.chevronRight, size: 20),
          ],
        ),
      ),
    );
  }
}
