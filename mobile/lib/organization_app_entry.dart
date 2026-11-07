import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/onboarding/self_hosted_entry_sheet.dart';
import 'features/pairing/pairing_page.dart';
import 'shared/auth/auth_provider.dart';
import 'shared/community/community.dart';
import 'shared/community/community_provider.dart';
import 'shared/hosted/account_identity_store.dart';
import 'shared/hosted/hosted_relay_auth.dart';
import 'shared/hosted/organization_environment.dart';
import 'shared/hosted/organization_sign_in.dart';
import 'shared/hosted/organization_identity.dart';

/// An internal build's landing: one "Sign in" that goes to the organization's
/// single sign-on, then joins the organization's community.
class OrganizationAppEntry extends HookConsumerWidget {
  const OrganizationAppEntry({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final signIn = ref.watch(organizationSignInProvider)!;
    useListenable(signIn);
    final joining = useState(false);
    final joinFailed = useState<_JoinFailure?>(null);
    // Set by a tap, so restoring a saved session never joins on its own.
    final joinRequested = useRef(false);
    final relay = 'wss://${OrganizationEnvironment.relayHosts.first}';

    Future<void> join() async {
      if (joining.value) return;
      joinRequested.value = false;
      joining.value = true;
      joinFailed.value = null;
      bool current() => context.mounted;
      final entry = ref.read(selfHostedEntryProvider);
      final account = ref.read(organizationAccountProvider)!;
      final storage = ref.read(communityStorageProvider);
      try {
        final bound = await organizationBoundPubkey(account);
        if (!current()) return;
        Community community;
        if (bound != null) {
          // The relay admits only the bound key: use this device's copy, or
          // the organization's backup of it. Never offer or create another.
          try {
            community = await entry.prepare(
              relay: relay,
              name: '',
              savedPubkey: bound,
              isCurrent: current,
            );
          } on AccountIdentityFailure catch (error) {
            if (error != AccountIdentityFailure.absent) rethrow;
            final nsec = await recoverOrganizationKey(account, bound);
            if (!current()) return;
            if (nsec == null) {
              joinFailed.value = _JoinFailure.notRestored;
              return;
            }
            community = await entry.prepare(
              relay: relay,
              name: '',
              nsec: nsec,
              isCurrent: current,
            );
          }
        } else {
          try {
            community = await entry.prepare(
              relay: relay,
              name: '',
              create: true,
              isCurrent: current,
            );
          } on AccountIdentityFailure catch (error) {
            // Saved identities exist: the member picks one or a new key, which
            // the organization binds at its first assertion.
            if (error != AccountIdentityFailure.absent || !context.mounted) {
              rethrow;
            }
            await showSelfHostedEntry(context, relay: relay, name: '');
            return;
          }
        }
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
          await entry.complete(community);
        }
      } catch (error) {
        debugPrint('Organization community entry failed: $error');
        if (context.mounted) joinFailed.value = _JoinFailure.failed;
      } finally {
        if (context.mounted) joining.value = false;
      }
    }

    // This landing shows only without the organization's community, so the
    // member has signed out: end the session rather than reuse it.
    useEffect(() {
      unawaited(signIn.signOut());
      return null;
    }, [signIn]);

    useEffect(() {
      if (joinRequested.value &&
          signIn.status == OrganizationSessionStatus.signedIn) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (context.mounted) unawaited(join());
        });
      }
      if (signIn.status == OrganizationSessionStatus.signedOut) {
        joinRequested.value = false;
      }
      return null;
    }, [signIn.status]);

    final busy =
        joining.value ||
        signIn.status == OrganizationSessionStatus.checking ||
        signIn.status == OrganizationSessionStatus.signingIn;
    final status =
        switch (joinFailed.value) {
          _JoinFailure.failed =>
            'Couldn’t connect to your organization. Try again.',
          _JoinFailure.notRestored =>
            'Couldn’t restore your organization identity on this device. '
                'Sign in on a device that has it, then try again.',
          null => null,
        } ??
        (signIn.failure != null ? 'Couldn’t sign in. Try again.' : null);

    return PairingPage(
      connectionLanding: true,
      showConnectionOptions: false,
      accountStatus: status,
      signInBusy: busy,
      onSignIn: () {
        if (signIn.status == OrganizationSessionStatus.signedIn) {
          unawaited(join());
          return;
        }
        joinRequested.value = true;
        joinFailed.value = null;
        unawaited(signIn.signIn());
      },
    );
  }
}

enum _JoinFailure { failed, notRestored }
