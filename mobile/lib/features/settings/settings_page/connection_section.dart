part of '../settings_page.dart';

class _ConnectionSection extends ConsumerWidget {
  const _ConnectionSection({required this.identityRecoveryPageBuilder});

  final WidgetBuilder identityRecoveryPageBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final config = ref.watch(relayConfigProvider);
    final authState = ref.watch(authProvider).value;
    final nsec = config.nsec;
    final community = authState?.community;

    return AppListCard(
      label: 'Connection',
      verticalPadding: Grid.twelve,
      children: [
        if (nsec != null && nsec.isNotEmpty && community != null) ...[
          _IdentityRow(nsec: nsec),
          AppListRow(
            icon: LucideIcons.scanQrCode,
            title: 'Send identity to desktop',
            subtitle: 'Scan a recovery code shown by Buzz Desktop',
            trailing: const _RowChevron(),
            onTap: () async {
              final pairing = ref.read(pairingProvider.notifier);
              final authorized = await pairing.authorizeIdentityExport(
                community: community,
              );
              if (!authorized) {
                if (!context.mounted) return;
                final message = ref.read(pairingProvider).errorMessage;
                if (message != null) {
                  ScaffoldMessenger.of(
                    context,
                  ).showSnackBar(SnackBar(content: Text(message)));
                }
                return;
              }

              try {
                if (!context.mounted) return;
                final resumed = await _waitForResumedFrame();
                if (!resumed) {
                  if (context.mounted) {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(
                        content: Text(
                          'Buzz did not return to the foreground. Try again.',
                        ),
                      ),
                    );
                  }
                  return;
                }
                if (!context.mounted) return;
                await Navigator.of(context).push(
                  MaterialPageRoute<void>(builder: identityRecoveryPageBuilder),
                );
              } finally {
                pairing.reset();
              }
            },
          ),
        ],
      ],
    );
  }
}

const _resumeWaitTimeout = Duration(seconds: 5);

Future<bool> _waitForResumedFrame() async {
  final binding = WidgetsBinding.instance;
  if (binding.lifecycleState != AppLifecycleState.resumed) {
    final resumed = Completer<void>();
    final listener = AppLifecycleListener(
      onResume: () {
        if (!resumed.isCompleted) resumed.complete();
      },
    );
    try {
      await resumed.future.timeout(_resumeWaitTimeout);
    } on TimeoutException {
      return false;
    } finally {
      listener.dispose();
    }
  }
  await binding.endOfFrame;
  return true;
}

/// Destructive, so it gets a container of its own rather than sitting at the
/// bottom of the connection group.
class _RemoveCommunitySection extends ConsumerWidget {
  const _RemoveCommunitySection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return AppListCard(
      verticalPadding: Grid.twelve,
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

    return Semantics(
      button: true,
      label: 'Copy identity public key',
      value: pubkey,
      child: AppListRow(
        icon: LucideIcons.key,
        title: 'Identity (pubkey)',
        trailing: Icon(
          LucideIcons.copy,
          size: 18,
          color: context.colors.onSurfaceVariant,
        ),
        onTap: () async {
          await copyToClipboard(context, pubkey, message: 'Pubkey copied');
        },
      ),
    );
  }
}

void _confirmRemoveCommunity(BuildContext context, WidgetRef ref) {
  showBuzzDialog<void>(
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
          onPressed: () async {
            Navigator.of(ctx).pop(); // close dialog
            try {
              await ref.read(authProvider.notifier).signOut();
            } catch (error) {
              if (!context.mounted) return;
              ScaffoldMessenger.of(context).showSnackBar(
                SnackBar(content: Text('Could not remove community: $error')),
              );
              return;
            }
            if (!context.mounted) return;
            // Pop all pushed routes back to root so MaterialApp.home rebuilds
            // to PairingPage when auth state changes.
            Navigator.of(context).popUntil((route) => route.isFirst);
          },
          style: FilledButton.styleFrom(backgroundColor: ctx.colors.error),
          child: const Text('Remove'),
        ),
      ],
    ),
  );
}
