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
import '../../shared/hosted/hosted_environment.dart';
import 'saved_identity_card.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/onboarding_step_page.dart';
import '../../shared/widgets/onboarding_loading_page.dart';
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
  bool fullScreen = false,
}) async {
  if (fullScreen) {
    return await Navigator.of(context, rootNavigator: true).push<bool>(
          MaterialPageRoute(
            builder: (_) => SelfHostedEntrySheet(
              relay: relay,
              name: name,
              fullScreen: true,
            ),
          ),
        ) ??
        false;
  }
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
        name: HostedEnvironment.communityDisplayName(
          name.isEmpty ? Uri.parse(relay).host : name,
          relay,
        ),
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
    this.fullScreen = false,
  });
  final String relay;
  final String name;
  final bool fullScreen;

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
          : TextButton(
              style: TextButton.styleFrom(
                foregroundColor: context.colors.onSurface,
                minimumSize: const Size.fromHeight(52),
                shape: const StadiumBorder(),
              ),
              onPressed: onPressed,
              child: child,
            );
    }

    final saved = choices.data?.choices ?? const <DeviceIdentityChoice>[];
    final identityTitle = saved.isEmpty
        ? 'Choose an identity'
        : saved.length == 1
        ? 'You already have a saved identity'
        : 'You already have saved identities';
    final identitySubtitle = saved.length == 1
        ? 'Use this identity or import a Nostr key.'
        : 'Choose an identity or import a Nostr key.';
    final identityCards = <Widget>[
      for (final identity in saved)
        SavedIdentityCard(
          identity: identity,
          onSelect: busy.value ? null : () => select(pubkey: identity.pubkey),
        ),
    ];
    final joinActions = <Widget>[
      if (choices.connectionState == ConnectionState.done) ...[
        button(
          onPressed: busy.value
              ? null
              : () async {
                  if (fullScreen) {
                    final imported = await Navigator.of(context).push<bool>(
                      MaterialPageRoute(
                        builder: (importContext) => NsecImportSheet(
                          fullScreen: true,
                          onBack: () {
                            generation.value++;
                            busy.value = false;
                            error.value = null;
                            Navigator.of(importContext).pop(false);
                          },
                          onImport: (nsec) => connect(nsec: nsec, close: false),
                        ),
                      ),
                    );
                    if (imported == true && context.mounted) {
                      Navigator.of(context).pop(true);
                    }
                  } else {
                    importFocusReady.value = false;
                    importing.value = true;
                  }
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
    ];
    final joinContent = Padding(
      padding: fullScreen ? EdgeInsets.zero : const EdgeInsets.all(Grid.gutter),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (choices.hasError || choices.data?.complete == false)
              const Text(
                'Some saved identities could not be read. You can use an available identity or import a key.',
              ),
            if (!fullScreen) ...[
              Text(identitySubtitle, style: context.textTheme.bodyMedium),
              const SizedBox(height: Grid.sm),
            ],
            // The full-screen page already provides 16 px below its subtitle.
            if (fullScreen && identityCards.isNotEmpty)
              const SizedBox(height: Grid.xxs),
            for (var i = 0; i < identityCards.length; i++) ...[
              if (i > 0) const SizedBox(height: Grid.xxs),
              identityCards[i],
            ],
            if (!fullScreen) ...[
              if (identityCards.isNotEmpty) const SizedBox(height: Grid.xxs),
              for (var i = 0; i < joinActions.length; i++) ...[
                if (i > 0) const SizedBox(height: Grid.xxs),
                joinActions[i],
              ],
            ],
          ],
        ),
      ),
    );
    if (fullScreen && busy.value) {
      return OnboardingLoadingPage(
        title: 'Getting ready',
        onCancel: () {
          generation.value++;
          busy.value = false;
        },
      );
    }
    if (fullScreen) {
      return OnboardingStepPage(
        title: identityTitle,
        subtitle: identitySubtitle,
        onBack: () {
          generation.value++;
          Navigator.of(context).pop(false);
        },
        content: joinContent,
        actions: joinActions,
      );
    }
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
        title: importing.value ? 'Import identity' : 'Choose an identity',
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
