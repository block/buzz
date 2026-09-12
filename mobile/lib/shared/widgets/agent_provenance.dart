import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../mentions/agent_identity_provider.dart';
import '../profile/user_cache_provider.dart';
import '../relay/relay.dart';
import '../theme/theme.dart';

/// Mobile has no local managed runtime inventory. Verified viewer-owned agents
/// are therefore not managed on this device; this says nothing about hosting.
final agentNotManagedHereProvider = Provider.family<bool, String>((
  ref,
  pubkey,
) {
  final viewer = ref.watch(myPubkeyProvider)?.toLowerCase();
  final key = pubkey.toLowerCase();
  final owners = ref.watch(agentOwnersProvider);
  // Cached display evidence cannot keep a retired authority feed affirmative.
  if (owners.isLoading || owners.hasError) return false;
  final profiles = ref.watch(userCacheProvider);
  final cache = ref.read(userCacheProvider.notifier);
  final owner = cache.profilePubkeys.contains(key)
      ? profiles[key]?.ownerPubkey
      : owners.asData?.value[key];
  return viewer != null && owner == viewer && key != viewer;
});

/// One accessible cloud marker, shared by mobile identity surfaces.
class AgentProvenance extends ConsumerWidget {
  final String? pubkey;
  final double size;
  const AgentProvenance({super.key, required this.pubkey, this.size = 12});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (pubkey == null || !ref.watch(agentNotManagedHereProvider(pubkey!))) {
      return const SizedBox.shrink();
    }
    const label = 'Not managed on this device';
    return Tooltip(
      message: label,
      excludeFromSemantics: true,
      child: Icon(
        LucideIcons.cloud,
        size: size,
        color: context.colors.primary,
        semanticLabel: label,
      ),
    );
  }
}
