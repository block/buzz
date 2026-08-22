import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community.dart';
import '../community/community_provider.dart';
import '../relay/relay_provider.dart';
import '../relay/relay_session.dart';
import '../relay/signed_event_relay.dart';
import 'dev_push_lease.dart';
import 'push_bridge.dart';
import 'push_subscription.dart';

/// Starts the push lifecycle only after authenticated relay connectivity and a
/// push-capable NIP-11 descriptor are both present.
class BuzzPushBootstrap extends HookConsumerWidget {
  const BuzzPushBootstrap({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(apnsDeviceToken);
    useListenable(pushAuthorizationGranted);
    final authorizationAttempt = useRef<String?>(null);
    final publicationAttempt = useRef<String?>(null);
    final session = ref.watch(relaySessionProvider);
    final config = ref.watch(relayConfigProvider);
    final community = ref.watch(activeCommunityProvider).value;
    final memberPubkey = ref.watch(myPubkeyProvider);

    useEffect(() {
      if (!_ready(session, config, community, memberPubkey)) return null;
      final attempt = '${community!.id}|${config.baseUrl}';
      if (authorizationAttempt.value == attempt) return null;
      authorizationAttempt.value = attempt;
      unawaited(
        _authorize(config.baseUrl).catchError((Object error, StackTrace stack) {
          authorizationAttempt.value = null;
          debugPrint('Push authorization bootstrap failed: $error');
          debugPrintStack(stackTrace: stack);
        }),
      );
      return null;
    }, [session.status, config.baseUrl, community?.id, memberPubkey]);

    final token = apnsDeviceToken.value;
    final authorized = pushAuthorizationGranted.value;
    useEffect(
      () {
        if (!_ready(session, config, community, memberPubkey) ||
            authorized != true ||
            token == null) {
          return null;
        }
        final state = community!.pushSubscriptionState;
        if (state.desired.isEmpty) return null;
        final desiredFingerprint = buzzPushSubscriptionsFingerprint(
          state.desired,
        );
        final acceptedFingerprint = state.accepted == null
            ? '-'
            : buzzPushSubscriptionsFingerprint(state.accepted!);
        final attempt = [
          community.id,
          config.baseUrl,
          token,
          desiredFingerprint,
          acceptedFingerprint,
          state.acceptedGeneration,
          state.acceptedGrantGeneration,
          state.acceptedInstallationId,
        ].join('|');
        if (publicationAttempt.value == attempt) return null;
        publicationAttempt.value = attempt;
        final relay = SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: config.nsec!,
        );
        unawaited(
          _publish(ref, config, community, memberPubkey!, relay).catchError((
            Object error,
            StackTrace stack,
          ) {
            publicationAttempt.value = null;
            debugPrint('Push lease bootstrap failed: $error');
            debugPrintStack(stackTrace: stack);
          }),
        );
        return null;
      },
      [
        session.status,
        config.baseUrl,
        community?.id,
        community?.pushSubscriptionState,
        memberPubkey,
        authorized,
        token,
      ],
    );

    return child;
  }

  static bool _ready(
    SessionState session,
    RelayConfig config,
    Community? community,
    String? memberPubkey,
  ) =>
      session.status == SessionStatus.connected &&
      community != null &&
      config.nsec != null &&
      config.nsec!.isNotEmpty &&
      memberPubkey != null &&
      memberPubkey.isNotEmpty;

  static Future<void> _authorize(String relayBaseUrl) async {
    await fetchBuzzPushLeaseDescriptor(relayBaseUrl);
    await requestBuzzPushAuthorization();
  }

  static Future<void> _publish(
    WidgetRef ref,
    RelayConfig config,
    Community community,
    String memberPubkey,
    SignedEventRelay relay,
  ) async {
    final state = community.pushSubscriptionState;
    final desired = state.desired;
    final desiredFingerprint = buzzPushSubscriptionsFingerprint(desired);
    final acceptedFingerprint = state.accepted == null
        ? null
        : buzzPushSubscriptionsFingerprint(state.accepted!);
    final descriptor = await fetchBuzzPushLeaseDescriptor(config.baseUrl);
    final grant = await enrollBuzzPush(
      config.wsUrl,
      Env.pushGatewayUrl,
      communitiesForSnapshotRefresh:
          ref.read(communityListProvider).value ?? [community],
    );
    if (state.authority == BuzzPushLeaseSubscriptionAuthority.accepted &&
        acceptedFingerprint == desiredFingerprint &&
        state.acceptedGrantGeneration == grant.generation &&
        state.acceptedInstallationId == grant.installationId) {
      return;
    }
    // Relay lease replacement and gateway delegation are independent state
    // machines. Subscription changes advance only the kind-30350 generation;
    // the opaque grant remains reusable until its own authority changes.
    final leaseGeneration = (state.acceptedGeneration ?? 0) + 1;

    await publishBuzzDevPushLeaseThroughRelay(
      grant: grant,
      leaseGeneration: leaseGeneration,
      descriptor: descriptor,
      nsec: config.nsec!,
      memberPubkey: memberPubkey,
      subscriptions: desired,
      relay: relay,
    );
    await ref
        .read(communityListProvider.notifier)
        .markPushLeaseAccepted(
          community.id,
          subscriptions: desired,
          generation: leaseGeneration,
          grantGeneration: grant.generation,
          installationId: grant.installationId,
        );
  }
}
