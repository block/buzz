import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../auth/auth_provider.dart';
import '../community/community.dart';
import '../community/community_provider.dart';
import '../deeplink/deep_link.dart';
import 'account_identity_store.dart';
import 'builderlab_account.dart';
import 'hosted_admission.dart';
import 'hosted_community_connection.dart';
import 'hosted_environment.dart';
import 'hosted_flow.dart';
import 'hosted_flow_provider.dart';
import 'hosted_relay_auth.dart';
import 'known_communities_sync.dart';

/// The app's known-communities sync owner. Null unless this build has a
/// Builderlab account; created once and kept for the app's lifetime.
final knownCommunitiesSyncProvider = Provider<KnownCommunitiesSync?>(
  (ref) => HostedEnvironment.enabled ? createKnownCommunitiesSync(ref) : null,
);

/// Wires the sync to the account, the saved communities, the hosted flow's
/// admission (sharing its relay [probe] seam) and the app lifecycle, then
/// starts it. The flow's own identity work must not interleave with a
/// background join, so joins wait until the flow is idle or complete.
KnownCommunitiesSync createKnownCommunitiesSync(
  Ref ref, {
  Future<void> Function(Community)? probe,
  KnownCommunitiesTimer? timer,
  Duration maxBackoff = const Duration(minutes: 1),
}) {
  final account = ref.read(builderlabAccountProvider);
  final flow = ref.read(hostedFlowProvider);
  final store = AccountIdentityStore();
  final admission = HostedAdmission(
    relayAuth: ref.read(hostedRelayAuthProvider),
    account: account,
    store: store,
    probe: probe,
  );
  bool settled() =>
      flow.stage == HostedFlowStage.idle ||
      flow.stage == HostedFlowStage.complete;
  final sync = KnownCommunitiesSync(
    account: account,
    outbox: ref.read(knownCommunitiesOutboxProvider),
    client: KnownCommunitiesClient(account),
    memberships: () async => [
      for (final community
          in await ref.read(communityStorageProvider).loadAll())
        community.relayUrl,
    ],
    canAdmit: settled,
    admit: (url, subject, isCurrent) => admitKnownCommunity(
      ref,
      url: url,
      subject: subject,
      store: store,
      admission: admission,
      isCurrent: isCurrent,
    ),
    timer: timer,
    maxBackoff: maxBackoff,
  );
  // Joins deferred while the flow owned the identity resume once it settles.
  var wasSettled = settled();
  void onFlow() {
    final now = settled();
    if (now && !wasSettled && sync.hasDeferredJoins) sync.kick();
    wasSettled = now;
  }

  flow.addListener(onFlow);
  final lifecycle = AppLifecycleListener(onResume: sync.kick);
  sync.start();
  ref.onDispose(() {
    flow.removeListener(onFlow);
    lifecycle.dispose();
    sync.dispose();
    admission.dispose();
  });
  return sync;
}

/// Joins a listed destination with the account's set-up identity, through the
/// same admission and persistence as an explicit Add, without the profile
/// prompt. It becomes the active community only when none is. An identity
/// that is not ready, or a saved community under another key, waits for the
/// next trigger ([KnownCommunitiesNotReady]); relay failures propagate.
Future<void> admitKnownCommunity(
  Ref ref, {
  required String url,
  required String subject,
  required AccountIdentityStore store,
  required HostedAdmission admission,
  required bool Function() isCurrent,
}) async {
  final snapshot = await store.forSubject(subject).read();
  if (snapshot == null ||
      !snapshot.hasBackupFor(
        backend: HostedEnvironment.origin.origin,
        subject: subject,
      )) {
    throw const KnownCommunitiesNotReady();
  }
  final storage = ref.read(communityStorageProvider);
  try {
    await connectHostedCommunity(
      link: AddCommunityDeepLink(relayUrl: url, name: Uri.parse(url).host),
      subject: subject,
      pubkey: snapshot.pubkey,
      isCurrent: isCurrent,
      storage: storage,
      admission: admission,
      persist: (community, current) async {
        final activeId = await storage.loadActiveId();
        if (!current()) return;
        await ref
            .read(authProvider.notifier)
            .authenticateWithCommunity(
              community,
              shouldContinue: current,
              activate: activeId == null,
            );
      },
      // A listed destination is an Add, never an invite, so there are no
      // starter channels to recover.
      recoverStarterChannels: (_, _) async {},
    );
  } on SavedCommunityIdentityConflict {
    throw const KnownCommunitiesNotReady();
  } on AccountIdentityFailure {
    throw const KnownCommunitiesNotReady();
  } on AccountFailure catch (failure) {
    if (const {
      AccountFailure.denied,
      AccountFailure.differentAccount,
      AccountFailure.signedOut,
      AccountFailure.accountSetupRequired,
    }.contains(failure)) {
      throw const KnownCommunitiesNotReady();
    }
    rethrow;
  }
}
