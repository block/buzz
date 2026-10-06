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

    if (nsec == null || nsec.isEmpty || community == null) {
      return const SizedBox.shrink();
    }

    return AppListCard(
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
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

class _IdentityRow extends StatelessWidget {
  const _IdentityRow({required this.nsec});

  final String nsec;

  @override
  Widget build(BuildContext context) {
    final privHex = nostr.Nip19.decode(payload: nsec).data;
    final npub = privHex.isNotEmpty
        ? fullNpub(nostr.Keys(privHex).public)
        : null;

    // The full npub is the canonical copy/share form (never raw hex); an
    // invalid identity is surfaced as unavailable and never copied.
    return Semantics(
      button: true,
      label: 'Copy identity public key',
      value: npub ?? 'Identity unavailable',
      child: AppListRow(
        title: 'Copy public key (npub)',
        trailing: Icon(
          LucideIcons.copy,
          size: 18,
          color: context.colors.onSurfaceVariant,
        ),
        onTap: npub == null
            ? null
            : () async {
                await copyToClipboard(
                  context,
                  npub,
                  message: 'Public key (npub) copied',
                );
                await successHaptic();
              },
      ),
    );
  }
}
