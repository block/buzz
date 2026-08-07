part of '../settings_page.dart';

class _ConnectionSection extends ConsumerWidget {
  const _ConnectionSection({required this.identityRecoveryPageBuilder});

  final WidgetBuilder identityRecoveryPageBuilder;

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
        if (nsec != null && nsec.isNotEmpty) ...[
          _IdentityRow(nsec: nsec),
          AppListRow(
            icon: LucideIcons.scanQrCode,
            title: 'Send identity to desktop',
            subtitle: 'Scan a recovery code shown by Buzz Desktop',
            trailing: const _RowChevron(),
            onTap: () async {
              final authorized = await ref
                  .read(pairingProvider.notifier)
                  .authorizeIdentityExport();
              if (!context.mounted) return;
              if (!authorized) {
                final message = ref.read(pairingProvider).errorMessage;
                if (message != null) {
                  ScaffoldMessenger.of(
                    context,
                  ).showSnackBar(SnackBar(content: Text(message)));
                }
                return;
              }
              await Navigator.of(context).push(
                MaterialPageRoute<void>(builder: identityRecoveryPageBuilder),
              );
              ref.read(pairingProvider.notifier).reset();
            },
          ),
        ],
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
  final communityName = ref.read(authProvider).value?.community?.name;
  final title = communityName == null || communityName.trim().isEmpty
      ? 'Remove community from this phone?'
      : 'Remove “$communityName” from this phone?';

  showBuzzDialog<void>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: Text(title),
      content: const Text(
        'You’ll be signed out of this community on this phone. '
        'To come back, you’ll need to add it again from another signed-in device.',
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
