part of '../settings_page.dart';

class _ConnectionSection extends ConsumerWidget {
  const _ConnectionSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final config = ref.watch(relayConfigProvider);
    final nsec = config.nsec;

    return AppListCard(
      label: 'Connection',
      children: [
        AppListRow(
          icon: LucideIcons.server,
          title: 'Connected to',
          subtitle: config.baseUrl,
        ),
        if (nsec != null && nsec.isNotEmpty) _IdentityRow(nsec: nsec),
      ],
    );
  }
}

/// Destructive, so it gets a container of its own rather than sitting at the
/// bottom of the connection group.
class _RemoveCommunitySection extends ConsumerWidget {
  const _RemoveCommunitySection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return AppListCard(
      children: [
        AppListRow(
          icon: LucideIcons.logOut,
          title: 'Remove community',
          titleColor: context.colors.error,
          onTap: () => _confirmRemoveCommunity(context, ref),
        ),
      ],
    );
  }
}

class _IdentityRow extends StatelessWidget {
  const _IdentityRow({required this.nsec});

  final String nsec;

  @override
  Widget build(BuildContext context) {
    final privHex = nostr.Nip19.decode(payload: nsec).data;
    final pubkey = privHex.isNotEmpty ? nostr.Keys(privHex).public : 'unknown';

    return AppListRow(
      icon: LucideIcons.key,
      title: 'Identity (pubkey)',
      subtitle: pubkey,
      subtitleStyle: context.textTheme.bodySmall?.copyWith(
        color: context.colors.onSurfaceVariant,
        fontFamily: 'GeistMono',
        fontSize: 11,
      ),
      subtitleMaxLines: 2,
      trailing: IconButton(
        icon: const Icon(LucideIcons.copy, size: 16),
        onPressed: () async {
          await copyToClipboard(context, pubkey, message: 'Pubkey copied');
        },
      ),
    );
  }
}

void _confirmRemoveCommunity(BuildContext context, WidgetRef ref) {
  showDialog<void>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text('Remove Community'),
      content: const Text(
        'This will disconnect this community. You will need '
        'to scan a new pairing code to reconnect.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () {
            Navigator.of(ctx).pop(); // close dialog
            // Pop all pushed routes back to root so MaterialApp.home rebuilds
            // to PairingPage when auth state changes.
            Navigator.of(context).popUntil((route) => route.isFirst);
            ref.read(authProvider.notifier).signOut();
          },
          style: FilledButton.styleFrom(backgroundColor: ctx.colors.error),
          child: const Text('Remove'),
        ),
      ],
    ),
  );
}
