import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/hosted/device_identity_source.dart';
import '../../shared/hosted/self_hosted_entry.dart';
import '../../shared/identity_transfer/nsec_import_sheet.dart';
import '../../shared/identity_transfer/native_community_entry.dart';
import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/buzz_titled_sheet_layout.dart';

/// Local preparation seam; the production service validates and retains keys.
final selfHostedEntryProvider = Provider<SelfHostedEntry>(
  (ref) => SelfHostedEntry(),
);

/// Uses an explicit identity choice without involving a hosted account.
Future<bool> showSelfHostedEntry(
  BuildContext context, {
  required String relay,
  required String name,
}) async {
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final container = ProviderScope.containerOf(context);
    final service = container.read(selfHostedEntryProvider);
    final auth = container.read(authProvider.notifier);
    final storage = container.read(communityStorageProvider);
    final dark = context.theme.brightness == Brightness.dark;
    final primary = context.colors.primary.toARGB32();
    // Keep the existing Flutter error state if saved identities are unreadable.
    List<DeviceIdentityChoice>? choices;
    try {
      final inventory = await service.choices();
      if (inventory.complete) choices = inventory.choices;
    } catch (_) {}
    if (!context.mounted) return false;
    if (choices != null) {
      final result = await NativeCommunityEntry.show(
        name: name.isEmpty ? Uri.parse(relay).host : name,
        dark: dark,
        primaryArgb: primary,
        choices: choices,
        connect: (pubkey, nsec, create, current) async {
          final community = await service.prepare(
            relay: relay,
            name: name,
            savedPubkey: pubkey,
            nsec: nsec,
            create: create,
            isCurrent: current,
          );
          if (!current()) return;
          await auth.authenticateWithCommunity(
            community,
            shouldContinue: current,
          );
          final saved = await storage.loadAll();
          if (saved.any(
            (item) => item.id == community.id && item.nsec == community.nsec,
          )) {
            await service.complete(community);
          }
        },
      );
      if (result != null) return result;
    }
    if (!context.mounted) return false;
  }
  return await showBuzzModalBottomSheet<bool>(
        context: context,
        showCloseButton: false,
        isScrollControlled: true,
        useRootNavigator: true,
        useSafeArea: true,
        // This sheet owns equal content gutters on all three lower edges.
        padBottom: false,
        builder: (_) => SelfHostedEntrySheet(relay: relay, name: name),
      ) ??
      false;
}

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
    final importing = useState(false);
    final importFocusReady = useState(false);
    final generation = useRef(0);
    final error = useState<String?>(null);
    bool mounted() => context.mounted;
    Future<void> connect({
      String? pubkey,
      String? nsec,
      bool create = false,
      bool close = true,
    }) async {
      if (busy.value) return;
      final attempt = ++generation.value;
      bool current() => mounted() && generation.value == attempt;
      busy.value = true;
      error.value = null;
      final storage = ref.read(communityStorageProvider);
      try {
        final community = await service.prepare(
          relay: relay,
          name: name,
          savedPubkey: pubkey,
          nsec: nsec,
          create: create,
          isCurrent: current,
        );
        if (!current()) return;
        await ref
            .read(authProvider.notifier)
            .authenticateWithCommunity(community, shouldContinue: current);
        // Admission can return early after cancellation. Check the durable
        // record before retiring the only retry copy of a generated key.
        final saved = await storage.loadAll();
        if (saved.any(
          (item) => item.id == community.id && item.nsec == community.nsec,
        )) {
          await service.complete(community);
        }
        if (close && current() && context.mounted) {
          Navigator.of(context).pop(true);
        }
      } catch (_) {
        if (current()) {
          error.value =
              'Could not join. Check this community accepts your identity, then try again. Your saved keys have been kept.';
        }
        rethrow;
      } finally {
        if (current()) busy.value = false;
      }
    }

    Future<void> select({String? pubkey, bool create = false}) async {
      try {
        await connect(pubkey: pubkey, create: create);
      } catch (_) {}
    }

    Widget button({
      required VoidCallback? onPressed,
      required Widget child,
      bool primary = false,
    }) {
      return primary
          ? FilledButton(
              style: FilledButton.styleFrom(
                backgroundColor: context.colors.primary,
                foregroundColor: context.colors.onPrimary,
                minimumSize: const Size.fromHeight(52),
                shape: const StadiumBorder(),
              ),
              onPressed: onPressed,
              child: child,
            )
          : OutlinedButton(
              style: OutlinedButton.styleFrom(
                foregroundColor: context.colors.onSurface,
                minimumSize: const Size.fromHeight(52),
                shape: const StadiumBorder(),
              ),
              onPressed: onPressed,
              child: child,
            );
    }

    final saved = choices.data?.choices ?? const <DeviceIdentityChoice>[];
    final joinContent = Padding(
      padding: const EdgeInsets.all(Grid.gutter),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              padding: const EdgeInsets.all(Grid.sm),
              decoration: BoxDecoration(
                color: context.theme.brightness == Brightness.dark
                    ? const Color(0xFF233039)
                    : Colors.white.withValues(alpha: 0.7),
                borderRadius: BorderRadius.circular(12),
                border: Border.all(
                  color: context.colors.primary.withValues(alpha: 0.15),
                ),
              ),
              child: Text(
                name.isEmpty ? Uri.parse(relay).host : name,
                textAlign: TextAlign.center,
                style: context.textTheme.titleMedium,
              ),
            ),
            const SizedBox(height: Grid.sm),
            if (choices.hasError || choices.data?.complete == false)
              const Text(
                'Some saved identities could not be read. You can use an available identity or import a key.',
              ),
            for (final identity in saved)
              button(
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
            if (choices.connectionState == ConnectionState.done) ...[
              button(
                onPressed: busy.value
                    ? null
                    : () {
                        importFocusReady.value = false;
                        importing.value = true;
                      },
                child: const Text('Import a Nostr key'),
              ),
              if (saved.isEmpty && choices.data?.complete == true)
                button(
                  primary: true,
                  onPressed: busy.value ? null : () => select(create: true),
                  child: const Text('Create an identity and join'),
                ),
              if (saved.isEmpty && choices.data?.complete == true)
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
    return AnimatedSize(
      duration: MediaQuery.disableAnimationsOf(context)
          ? Duration.zero
          : const Duration(milliseconds: 250),
      curve: Curves.easeOutCubic,
      alignment: Alignment.bottomCenter,
      onEnd: () {
        if (context.mounted && importing.value) importFocusReady.value = true;
      },
      child: BuzzTitledSheetLayout(
        title: importing.value ? 'Import identity' : 'Join community',
        onClose: importing.value
            ? () {
                generation.value++;
                FocusManager.instance.primaryFocus?.unfocus();
                busy.value = false;
                error.value = null;
                importFocusReady.value = false;
                importing.value = false;
              }
            : null,
        child: importing.value
            ? SizedBox(
                height: MediaQuery.sizeOf(context).height * 0.8,
                child: NsecImportSheet(
                  autofocus:
                      importFocusReady.value ||
                      MediaQuery.disableAnimationsOf(context),
                  onImport: (nsec) => connect(nsec: nsec, close: false),
                ),
              )
            : joinContent,
      ),
    );
  }
}
