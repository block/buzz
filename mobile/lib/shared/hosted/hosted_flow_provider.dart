import 'dart:async';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../features/invites/invite_join_provider.dart';
import '../../features/profile/profile_provider.dart';
import '../relay/relay_provider.dart';
import '../relay/relay_session.dart';
import '../auth/auth.dart';
import '../community/paired_community_landing.dart';
import '../deeplink/deep_link.dart';
import 'account_identity_store.dart';
import 'browser_auth.dart';
import 'builderlab_account.dart';
import 'hosted_admission.dart';
import 'hosted_community_connection.dart';
import 'hosted_flow.dart';
import 'hosted_identity_service.dart';
import 'hosted_relay_auth.dart';

// Set by a community-entry surface while it owns setup presentation.
final hostedInlineEntryProvider = NotifierProvider<HostedInlineEntry, bool>(
  HostedInlineEntry.new,
);

class HostedInlineEntry extends Notifier<bool> {
  @override
  bool build() => false;

  void setActive(bool active) {
    if (ref.mounted) state = active;
  }
}

/// Signals a completed local admission to the currently mounted Add route.
final communityEntryCompletionProvider =
    NotifierProvider<CommunityEntryCompletion, int>(
      CommunityEntryCompletion.new,
    );

class CommunityEntryCompletion extends Notifier<int> {
  @override
  int build() => 0;

  void complete() {
    if (ref.mounted) state++;
  }
}

final hostedFlowProvider = Provider<HostedFlow>((ref) => createHostedFlow(ref));

/// Shared orchestration with injectable browser and relay network boundaries.
HostedFlow createHostedFlow(
  Ref ref, {
  BrowserAuth? browserAuth,
  Future<void> Function(Community)? probe,
}) {
  final account = ref.read(builderlabAccountProvider);
  final browser = browserAuth ?? BrowserAuth();
  final store = AccountIdentityStore();
  late final HostedFlow flow;
  final admission = HostedAdmission(
    relayAuth: ref.read(hostedRelayAuthProvider),
    account: account,
    store: store,
    confirmPolicy: (policy) => flow.confirmJoinPolicy(policy),
    probe: probe,
  );
  flow = HostedFlow(
    account: account,
    browser: browser,
    store: store,
    identity: HostedIdentityService(account: account, store: store),
    enterCommunity: (link, subject, pubkey, isCurrent) async {
      await connectHostedCommunity(
        link: link,
        subject: subject,
        pubkey: pubkey,
        isCurrent: () => isCurrent() && account.account?.subject == subject,
        storage: ref.read(communityStorageProvider),
        admission: admission,
        persist: (community, current) => ref
            .read(authProvider.notifier)
            .authenticateWithCommunity(
              flow.pairedSensitiveActionPolicy == null
                  ? community
                  : community.copyWith(
                      sensitiveActionPolicy: flow.pairedSensitiveActionPolicy,
                    ),
              shouldContinue: current,
              activate:
                  community.starterSetupIncomplete ||
                  flow.addedCommunityCount == 0,
            ),
        recoverStarterChannels: (community, current) =>
            finishHostedStarterSetup(ref, community, current),
      );
      if (flow.addedCommunityCount == 0 && isCurrent()) {
        await offerHostedProfile(
          ref,
          flow,
          link,
          pubkey,
          () => isCurrent() && account.account?.subject == subject,
        );
      }
    },
  );
  void onCompleted() {
    if (flow.reconnecting && flow.stage == HostedFlowStage.complete) {
      unawaited(ref.read(relaySessionProvider.notifier).reconnect());
    } else if (flow.accountOnly && flow.stage == HostedFlowStage.complete) {
      ref.read(communityGridLandingProvider.notifier).request();
    }
  }

  flow.addListener(onCompleted);
  ref.onDispose(() {
    flow.removeListener(onCompleted);
    flow.dispose();
    browser.dispose();
    admission.dispose();
  });
  return flow;
}

/// Optional and scoped to the first activated community. A failed read is not
/// proof that no profile exists; leave it to Settings in that case.
Future<void> offerHostedProfile(
  Ref ref,
  HostedFlow flow,
  BuzzDeepLink link,
  String pubkey,
  bool Function() isCurrent,
) async {
  final relay = link.relayUrl;
  if (relay == null || !ref.mounted || !isCurrent()) return;
  final active = await ref.read(activeCommunityProvider.future);
  bool current() {
    if (!ref.mounted || !isCurrent()) return false;
    final now = ref.read(activeCommunityProvider).asData?.value;
    return now != null &&
        now.id == active?.id &&
        now.pubkey == pubkey &&
        relayIdentity(now.relayUrl) == relayIdentity(relay);
  }

  if (!current()) return;
  // Keep the provider hydrated throughout the optional sheet.
  final subscription = ref.listen(profileProvider, (_, _) {});
  try {
    try {
      final profile = await ref.read(profileProvider.future);
      if (profile != null || !current()) return;
    } catch (_) {
      return;
    }
    await flow.requestProfile(
      (name) async {
        if (!current()) throw AccountFailure.unavailable;
        await ref.read(profileProvider.notifier).updateDisplayName(name);
      },
      saveDraft: (name, prepareAvatar) async {
        if (!current()) throw AccountFailure.unavailable;
        final writer = ref.read(profileProvider.notifier);
        final avatar = await prepareAvatar?.call();
        if (!current()) throw AccountFailure.unavailable;
        if (avatar == null) {
          await writer.updateDisplayName(name);
        } else {
          await writer.updateSetup(name, avatar);
        }
      },
    );
  } finally {
    subscription.close();
  }
}

/// Completes the existing scoped invite recovery only after activation. The
/// pending flag remains durable on failure, cancellation or a community switch.
Future<void> finishHostedStarterSetup(
  Ref ref,
  Community community,
  bool Function() isCurrent,
) async {
  void checkCurrent() {
    if (!ref.mounted || !isCurrent()) throw AccountFailure.unavailable;
  }

  bool matches(Community? candidate) =>
      candidate?.id == community.id &&
      candidate?.relayUrl == community.relayUrl &&
      candidate?.pubkey == community.pubkey &&
      candidate?.nsec == community.nsec;

  checkCurrent();
  final active = await ref.read(activeCommunityProvider.future);
  checkCurrent();
  if (!matches(active)) throw AccountFailure.unavailable;
  final config = RelayConfig(baseUrl: community.relayUrl, nsec: community.nsec);
  await ref
      .read(inviteJoinRecoveryProvider)(
        InviteJoinRecoveryScope(
          relayHttpOrigin: config.baseUrl,
          nsec: config.nsec,
        ),
      )
      .ensureStarterChannels();
  checkCurrent();
  await ref.read(communityTransitionProvider).runExclusive(() async {
    checkCurrent();
    final storage = ref.read(communityStorageProvider);
    final saved = (await storage.loadAll())
        .where((item) => item.id == community.id)
        .firstOrNull;
    final activeId = await storage.loadActiveId();
    checkCurrent();
    if (!matches(saved) || activeId != community.id) {
      throw AccountFailure.unavailable;
    }
    await storage.save(saved!.copyWith(starterSetupIncomplete: false));
    await syncStoredCommunitySnapshot(ref);
    ref.invalidate(communityListProvider);
    ref.invalidate(activeCommunityProvider);
    ref.invalidate(authProvider);
  });
}
