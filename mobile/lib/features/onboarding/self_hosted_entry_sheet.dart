import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/hosted/device_identity_source.dart';
import '../../shared/hosted/self_hosted_entry.dart';
import '../../shared/identity_transfer/nsec_import_sheet.dart';
import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/modal_presentation.dart';

/// Local preparation seam; the production service validates and retains keys.
final selfHostedEntryProvider = Provider<SelfHostedEntry>(
  (ref) => SelfHostedEntry(),
);

/// Uses an explicit identity choice without involving a hosted account.
Future<bool> showSelfHostedEntry(
  BuildContext context, {
  required String relay,
  required String name,
}) async =>
    await showBuzzModalBottomSheet<bool>(
      context: context,
      title: 'Join community',
      isScrollControlled: true,
      useRootNavigator: true,
      useSafeArea: true,
      builder: (_) => SelfHostedEntrySheet(relay: relay, name: name),
    ) ??
    false;

class SelfHostedEntrySheet extends HookConsumerWidget {
  const SelfHostedEntrySheet({
    super.key,
    required this.relay,
    required this.name,
  });
  final String relay;
  final String name;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final service = ref.watch(selfHostedEntryProvider);
    final choices = useFuture(useMemoized(service.choices));
    final busy = useState(false);
    final error = useState<String?>(null);
    bool mounted() => context.mounted;
    Future<void> connect({
      String? pubkey,
      String? nsec,
      bool create = false,
      bool close = true,
    }) async {
      if (busy.value) return;
      busy.value = true;
      error.value = null;
      try {
        final community = await service.prepare(
          relay: relay,
          name: name,
          savedPubkey: pubkey,
          nsec: nsec,
          create: create,
          isCurrent: mounted,
        );
        if (!mounted()) return;
        await ref
            .read(authProvider.notifier)
            .authenticateWithCommunity(community, shouldContinue: mounted);
        if (close && context.mounted) Navigator.of(context).pop(true);
      } catch (_) {
        if (mounted()) {
          error.value =
              'Could not join. Check this community accepts your identity, then try again. Your saved keys have been kept.';
        }
        rethrow;
      } finally {
        if (mounted()) busy.value = false;
      }
    }

    Future<void> select({String? pubkey, bool create = false}) async {
      try {
        await connect(pubkey: pubkey, create: create);
      } catch (_) {}
    }

    final saved = choices.data ?? const <DeviceIdentityChoice>[];
    return Padding(
      padding: const EdgeInsets.all(Grid.gutter),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(Uri.parse(relay).host, style: context.textTheme.titleMedium),
            const SizedBox(height: Grid.sm),
            const Text(
              'Choose the identity to use in this community. Your key stays on this device; Builderlab is not required.',
            ),
            const SizedBox(height: Grid.sm),
            if (choices.hasError)
              const Text(
                'Saved identities could not be read. Unlock your device and try again.',
              ),
            for (final identity in saved)
              OutlinedButton(
                onPressed: busy.value
                    ? null
                    : () => select(pubkey: identity.pubkey),
                child: Column(
                  children: [
                    Text(identity.label),
                    Text(fullNpub(identity.pubkey) ?? identity.pubkey),
                  ],
                ),
              ),
            if (choices.connectionState == ConnectionState.done &&
                !choices.hasError) ...[
              OutlinedButton(
                onPressed: busy.value
                    ? null
                    : () async {
                        final imported = await showNsecImportSheet(
                          context,
                          onImport: (nsec) => connect(nsec: nsec, close: false),
                        );
                        if (imported && context.mounted) {
                          Navigator.of(context).pop(true);
                        }
                      },
                child: const Text('Import a Nostr key'),
              ),
              if (saved.isEmpty)
                FilledButton(
                  onPressed: busy.value ? null : () => select(create: true),
                  child: const Text('Create an identity and join'),
                ),
              if (saved.isEmpty)
                const Text(
                  'Export a backup in Settings after joining. This device will hold your only copy until you back it up.',
                ),
            ],
            if (busy.value)
              const Padding(
                padding: EdgeInsets.all(Grid.sm),
                child: Text('Joining…'),
              ),
            if (error.value != null)
              Text(error.value!, style: TextStyle(color: context.colors.error)),
          ],
        ),
      ),
    );
  }
}
