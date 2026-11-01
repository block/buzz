import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/buzz_icons.dart';
import '../theme/theme.dart';
import '../verified_profiles/verified_profile_directory.dart';
import 'verified_profile_details.dart';

/// Interface chrome beside a name: shown only for a NIP-FI verified profile
/// under a successful discovery, reconciled with the relay on this connection.
/// A cached record shows no mark. Never derived from kind 0 or NIP-05.
class VerifiedMark extends ConsumerWidget {
  const VerifiedMark({super.key, required this.pubkey, this.size = 14});

  final String pubkey;
  final double size;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profile = ref.watch(
      verifiedProfilesProvider.select((all) => all[pubkey.toLowerCase()]),
    );
    if (profile == null || !profile.checked) return const SizedBox.shrink();
    final label = verifiedByLabel(profile.issuerLabel);
    return Semantics(
      label: label,
      child: Tooltip(
        message: label,
        // The Semantics above owns the label, so it is read once.
        excludeFromSemantics: true,
        child: Padding(
          padding: const EdgeInsets.only(left: Grid.half),
          child: ExcludeSemantics(
            child: Icon(
              BuzzIcons.circleCheck,
              key: const ValueKey('verified-mark'),
              size: size * MediaQuery.textScalerOf(context).scale(1),
              color: context.colors.primary,
            ),
          ),
        ),
      ),
    );
  }
}
