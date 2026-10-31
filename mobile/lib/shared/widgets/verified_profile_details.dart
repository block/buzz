import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';
import '../verified_profiles/verified_profile_directory.dart';

/// "Last verified" wording for a historical record: never current access.
String lastVerifiedLabel(int verifiedAt, DateTime now) {
  final seconds = now
      .difference(DateTime.fromMillisecondsSinceEpoch(verifiedAt * 1000))
      .inSeconds;
  if (seconds < 60) return 'Last verified just now';
  final (count, unit) = switch (seconds) {
    < 3600 => (seconds ~/ 60, 'minute'),
    < 86400 => (seconds ~/ 3600, 'hour'),
    < 2592000 => (seconds ~/ 86400, 'day'),
    < 31536000 => (seconds ~/ 2592000, 'month'),
    _ => (seconds ~/ 31536000, 'year'),
  };
  return 'Last verified $count $unit${count == 1 ? '' : 's'} ago';
}

/// Where a verification is inspectable: issuer, full name, last verified time,
/// and whether the record was reconciled with the relay or is cached.
class VerifiedProfileDetails extends ConsumerWidget {
  const VerifiedProfileDetails({super.key, required this.pubkey, this.now});

  final String pubkey;
  final DateTime? now;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profile = ref.watch(
      verifiedProfilesProvider.select((all) => all[pubkey.toLowerCase()]),
    );
    if (profile == null) return const SizedBox.shrink();
    final style = context.textTheme.bodySmall?.copyWith(
      color: context.colors.onSurfaceVariant,
    );
    final lines = [
      if (profile.fullName != null) profile.fullName!,
      'Verified by ${profile.issuerLabel}',
      '${lastVerifiedLabel(profile.verifiedAt, now ?? DateTime.now())}'
          '${profile.checked ? '' : ' (cached)'}',
    ];
    return Padding(
      padding: const EdgeInsets.only(top: Grid.xxs),
      child: Column(
        key: const ValueKey('verified-profile-details'),
        children: [
          for (final line in lines)
            Text(line, textAlign: TextAlign.center, style: style),
        ],
      ),
    );
  }
}
