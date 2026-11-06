import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/onboarding/self_hosted_entry_sheet.dart';
import 'features/pairing/pairing_page.dart';
import 'shared/auth/auth_provider.dart';
import 'shared/community/community.dart';
import 'shared/hosted/account_identity_store.dart';
import 'shared/hosted/hosted_relay_auth.dart';
import 'shared/hosted/organization_environment.dart';
import 'shared/hosted/organization_sign_in.dart';
import 'shared/hosted/self_hosted_entry.dart';
import 'shared/relay/relay_provider.dart';
import 'shared/relay/relay_socket.dart';

/// Joins the organization's community with the build's session assertion.
final organizationEntryProvider = Provider<SelfHostedEntry>((ref) {
  final auth = ref.watch(hostedRelayAuthProvider);
  return SelfHostedEntry(probe: (community) => _probe(community, auth));
});

/// An internal build's landing: one "Sign in" that goes to the organization's
/// single sign-on, then joins the organization's community.
class OrganizationAppEntry extends HookConsumerWidget {
  const OrganizationAppEntry({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final signIn = ref.watch(organizationSignInProvider)!;
    useListenable(signIn);
    final joining = useState(false);
    final joinFailed = useState(false);
    // Set by a tap, so restoring a saved session never joins on its own.
    final joinRequested = useRef(false);
    final relay = 'wss://${OrganizationEnvironment.relayHosts.first}';

    Future<void> join() async {
      if (joining.value) return;
      joinRequested.value = false;
      joining.value = true;
      joinFailed.value = false;
      bool current() => context.mounted;
      final entry = ref.read(organizationEntryProvider);
      try {
        final Community community;
        try {
          community = await entry.prepare(
            relay: relay,
            name: '',
            create: true,
            isCurrent: current,
          );
        } on AccountIdentityFailure catch (error) {
          // Saved identities exist: the member picks one or a new key.
          if (error != AccountIdentityFailure.absent || !context.mounted) {
            rethrow;
          }
          await showSelfHostedEntry(context, relay: relay, name: '');
          return;
        }
        if (!current()) return;
        await ref
            .read(authProvider.notifier)
            .authenticateWithCommunity(community, shouldContinue: current);
        await entry.complete(community);
      } catch (error) {
        debugPrint('Organization community entry failed: $error');
        if (context.mounted) joinFailed.value = true;
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
    final status = joinFailed.value
        ? 'Couldn’t connect to your organization. Try again.'
        : signIn.failure != null
        ? 'Couldn’t sign in. Try again.'
        : null;

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
        joinFailed.value = false;
        unawaited(signIn.signIn());
      },
    );
  }
}

/// Entry must authenticate before the community becomes active, and an
/// organization relay refuses the upgrade without the session's assertion.
Future<void> _probe(Community community, HostedRelayAuth auth) async {
  final socket = RelaySocket(
    wsUrl: community.relayUrl,
    nsec: community.nsec,
    onMessage: (_) {},
    onConnected: () {},
    onDisconnected: (_) {},
  );
  try {
    socket.headers = await auth.headers(
      RelayConfig(baseUrl: community.relayUrl, nsec: community.nsec),
    );
    await socket.connect().timeout(const Duration(seconds: 20));
    if (socket.state != SocketState.connected) {
      throw StateError('Community refused entry');
    }
  } finally {
    socket.dispose();
  }
}
