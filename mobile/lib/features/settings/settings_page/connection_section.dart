part of '../settings_page.dart';

class _ConnectionSection extends ConsumerWidget {
  const _ConnectionSection({required this.identityRecoveryPageBuilder});

  final WidgetBuilder identityRecoveryPageBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final config = ref.watch(relayConfigProvider);
    final activeCommunity = ref.watch(activeCommunityProvider).value;
    final nsec = config.nsec;

    return AppListCard(
      label: 'Connection',
      children: [
        AppListRow(
          icon: LucideIcons.server,
          title: 'Connected to',
          subtitle: config.baseUrl,
        ),
        AppListRow(
          icon: LucideIcons.network,
          title: 'Campus / LAN relay',
          subtitle: config.lanRelayUrl ?? 'Not configured',
          trailing: const _RowChevron(),
          onTap: activeCommunity == null
              ? null
              : () => showBuzzDialog<void>(
                  context: context,
                  builder: (_) => _LanRelayDialog(
                    communityId: activeCommunity.id,
                    initialValue: activeCommunity.lanRelayUrl,
                  ),
                ),
        ),
        if (nsec != null && nsec.isNotEmpty) ...[
          _IdentityRow(nsec: nsec),
          AppListRow(
            icon: LucideIcons.scanQrCode,
            title: 'Send identity to desktop',
            subtitle: 'Scan a recovery code shown by Buzz Desktop',
            trailing: const _RowChevron(),
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: identityRecoveryPageBuilder),
            ),
          ),
        ],
      ],
    );
  }
}

class _LanRelayDialog extends HookConsumerWidget {
  const _LanRelayDialog({
    required this.communityId,
    required this.initialValue,
  });

  final String communityId;
  final String? initialValue;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController(text: initialValue ?? '');
    final error = useState<String?>(null);
    final isSaving = useState(false);

    Future<void> save() async {
      String? normalized;
      try {
        normalized = normalizeLanRelayUrl(controller.text);
      } on FormatException catch (exception) {
        error.value = exception.message;
        return;
      }

      isSaving.value = true;
      await ref
          .read(communityListProvider.notifier)
          .updateLanRelayUrl(communityId, normalized);
      if (context.mounted) Navigator.of(context).pop();
    }

    return AlertDialog(
      title: const Text('Campus / LAN relay'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          TextField(
            controller: controller,
            enabled: !isSaving.value,
            keyboardType: TextInputType.url,
            autocorrect: false,
            enableSuggestions: false,
            decoration: InputDecoration(
              labelText: 'Private relay address',
              hintText: 'ws://10.0.0.8:3000',
              errorText: error.value,
            ),
            onChanged: (_) => error.value = null,
            onSubmitted: (_) {
              if (!isSaving.value) unawaited(save());
            },
          ),
          const SizedBox(height: Grid.xs),
          Text(
            'Buzz tries this private address first, then falls back to the '
            'public relay. Leave it empty to disable the fast path.',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: isSaving.value ? null : () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: isSaving.value ? null : () => unawaited(save()),
          child: const Text('Save'),
        ),
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
