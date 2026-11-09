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
/// admission (sharing its relay [probe] seam), the community entry surface
/// and the app lifecycle, then starts it. The flow's own identity work must
/// not interleave with a background join, so joins wait until the flow is
/// idle or complete; what the user leaves unchosen in the flow's picker is
/// declined for this sign-in. A join that would open the first community
/// does so only while the entry surface is idle; otherwise the community
/// opens once the surface is idle again.
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
  // A join made while the entry surface was busy left no community open.
  var openingDeferred = false;
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
      canOpen: () => ref.read(communityEntryIdleProvider),
      onOpeningDeferred: () => openingDeferred = true,
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
  flow.onCommunitiesDeclined = sync.decline;
  ref.listen(communityEntryIdleProvider, (_, idle) {
    if (!idle || !openingDeferred) return;
    openingDeferred = false;
    unawaited(
      openFirstSavedCommunity(ref).catchError((Object error) {
        debugPrint('Known-communities sync could not open a community: $error');
      }),
    );
  });
  final lifecycle = AppLifecycleListener(onResume: sync.kick);
  sync.start();
  ref.onDispose(() {
    flow.removeListener(onFlow);
    if (identical(flow.onCommunitiesDeclined, sync.decline)) {
      flow.onCommunitiesDeclined = null;
    }
    lifecycle.dispose();
    sync.dispose();
    admission.dispose();
  });
  return sync;
}

/// Opens the first saved community when none is open, as a join made while
/// the entry surface was busy left it. Nothing happens once one is open.
Future<void> openFirstSavedCommunity(Ref ref) async {
  final storage = ref.read(communityStorageProvider);
  if (await storage.loadActiveId() != null) return;
  final saved = await storage.loadAll();
  if (saved.isEmpty) return;
  await ref
      .read(communityListProvider.notifier)
      .switchCommunity(saved.first.id);
}

/// Joins a listed destination with the account's set-up identity, through the
/// same admission and persistence as an explicit Add, without the profile
/// prompt. It becomes the open community only when none is and [canOpen]
/// holds (the entry surface is idle); otherwise it is saved unopened and
/// [onOpeningDeferred] is told. An identity that is not ready, or a session
/// that ended or changed, waits for the next trigger
/// ([KnownCommunitiesNotReady]); a community saved here under another key,
/// or a relay that refuses this identity, fails this destination alone
/// ([KnownCommunityAdmissionDenied]); other relay failures propagate.
Future<void> admitKnownCommunity(
  Ref ref, {
  required String url,
  required String subject,
  required AccountIdentityStore store,
  required HostedAdmission admission,
  required bool Function() isCurrent,
  required bool Function() canOpen,
  required void Function() onOpeningDeferred,
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
        final open = activeId == null && canOpen();
        await ref
            .read(authProvider.notifier)
            .authenticateWithCommunity(
              community,
              shouldContinue: current,
              activate: open,
            );
        if (activeId == null && !open && current()) onOpeningDeferred();
      },
      // A listed destination is an Add, never an invite, so there are no
      // starter channels to recover.
      recoverStarterChannels: (_, _) async {},
    );
  } on SavedCommunityIdentityConflict {
    throw const KnownCommunityAdmissionDenied();
  } on AccountIdentityFailure {
    throw const KnownCommunitiesNotReady();
  } on AccountFailure catch (failure) {
    if (failure == AccountFailure.denied) {
      throw const KnownCommunityAdmissionDenied();
    }
    if (const {
      AccountFailure.differentAccount,
      AccountFailure.signedOut,
      AccountFailure.accountSetupRequired,
    }.contains(failure)) {
      throw const KnownCommunitiesNotReady();
    }
    rethrow;
  }
}
