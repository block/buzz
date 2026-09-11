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
import 'push_lease_revocation_outbox.dart';
import 'push_relay_capability_provider.dart';
import 'push_subscription.dart';

const _pushBootstrapRetryDelay = Duration(seconds: 5);

@visibleForTesting
class BuzzPushAttemptGate {
  BuzzPushAttemptGate({this.retryDelay = _pushBootstrapRetryDelay});

  final Duration retryDelay;
  String? _attempt;
  Timer? _retryTimer;

  bool tryBegin(String attempt) {
    if (_attempt == attempt) return false;
    _retryTimer?.cancel();
    _retryTimer = null;
    _attempt = attempt;
    return true;
  }

  void failed(String attempt, {required VoidCallback retry}) {
    if (_attempt != attempt) return;
    _attempt = null;
    _retryTimer?.cancel();
    _retryTimer = Timer(retryDelay, () {
      _retryTimer = null;
      if (_attempt == null) retry();
    });
  }

  void retryAfter(
    String attempt, {
    required Duration delay,
    required VoidCallback retry,
  }) {
    if (_attempt != attempt) return;
    _retryTimer?.cancel();
    _retryTimer = Timer(delay, () {
      _retryTimer = null;
      if (_attempt != attempt) return;
      _attempt = null;
      retry();
    });
  }

  void complete(String attempt) {
    if (_attempt != attempt) return;
    _retryTimer?.cancel();
    _retryTimer = null;
    _attempt = null;
  }

  void dispose() => _retryTimer?.cancel();
}

@visibleForTesting
String buzzPushPublicationAttemptKey({
  required String communityId,
  required String relayBaseUrl,
  required String token,
  required BuzzPushLeaseDescriptor descriptor,
  required List<BuzzPushSubscription> subscriptions,
}) => [
  communityId,
  relayBaseUrl,
  token,
  descriptor.executorKeyId,
  descriptor.executorPubkey,
  buzzPushSubscriptionsFingerprint(subscriptions),
].join('|');

@visibleForTesting
bool buzzPushLifecycleEnabled({
  required Community? community,
  required BuzzPushLeaseDescriptor? descriptor,
}) => community?.pushNotificationsEnabled == true && descriptor != null;

@visibleForTesting
Future<int> publishBuzzPushLeaseRecoverably({
  required Future<int> Function() reserveGeneration,
  required Future<void> Function(int generation) publish,
  required Future<void> Function(int generation) markAccepted,
}) async {
  final generation = await reserveGeneration();
  await publish(generation);
  await markAccepted(generation);
  return generation;
}

/// Starts the push lifecycle only after authenticated relay connectivity and a
/// push-capable NIP-11 descriptor are both present.
class BuzzPushBootstrap extends HookConsumerWidget {
  const BuzzPushBootstrap({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(apnsDeviceToken);
    final registrationAttempt = useMemoized(BuzzPushAttemptGate.new);
    final publicationAttempt = useMemoized(BuzzPushAttemptGate.new);
    final tombstoneAttempt = useMemoized(BuzzPushAttemptGate.new);
    final registrationRetry = useState(0);
    final publicationRetry = useState(0);
    final tombstoneRetry = useState(0);
    final revocationOutbox = ref.watch(buzzPushLeaseRevocationOutboxProvider);
    final session = ref.watch(relaySessionProvider);
    final communities = ref.watch(communityListProvider).value ?? const [];
    final config = ref.watch(relayConfigProvider);
    final community = ref.watch(activeCommunityProvider).value;
    final memberPubkey = ref.watch(myPubkeyProvider);
    final descriptor = ref.watch(currentRelayPushDescriptorProvider).value;

    useEffect(() {
      final listener = AppLifecycleListener(
        onResume: () => _runRevocationOutbox(revocationOutbox.trigger),
      );
      _runRevocationOutbox(revocationOutbox.start);
      return listener.dispose;
    }, [revocationOutbox]);

    useEffect(() {
      if (session.status == SessionStatus.connected) {
        _runRevocationOutbox(revocationOutbox.trigger);
      }
      return null;
    }, [revocationOutbox, session.status]);

    useEffect(
      () => () {
        registrationAttempt.dispose();
        publicationAttempt.dispose();
        tombstoneAttempt.dispose();
      },
      const [],
    );

    useEffect(
      () {
        final pendingCommunities = communities
            .where(
              (candidate) =>
                  !candidate.pushNotificationsEnabled &&
                  candidate.pushSubscriptionState.pendingTombstoneGeneration !=
                      null,
            )
            .toList();
        if (session.status != SessionStatus.connected ||
            pendingCommunities.isEmpty) {
          return null;
        }
        const attempt = 'pending-tombstones';
        if (!tombstoneAttempt.tryBegin(attempt)) return null;
        unawaited(() async {
          try {
            Object? firstError;
            StackTrace? firstStack;
            for (final pendingCommunity in pendingCommunities) {
              try {
                await ref
                    .read(communityListProvider.notifier)
                    .retryPendingPushLeaseTombstone(
                      pendingCommunity.id,
                      advanceGeneration: true,
                    );
              } catch (error, stack) {
                firstError ??= error;
                firstStack ??= stack;
              }
            }
            if (firstError != null) {
              Error.throwWithStackTrace(firstError, firstStack!);
            }
            tombstoneAttempt.complete(attempt);
          } catch (error, stack) {
            tombstoneAttempt.failed(
              attempt,
              retry: () {
                if (context.mounted) tombstoneRetry.value += 1;
              },
            );
            debugPrint('Push lease tombstone retry failed: $error');
            debugPrintStack(stackTrace: stack);
          }
        }());
        return null;
      },
      [
        session.status,
        for (final candidate in communities)
          '${candidate.id}|${candidate.pushNotificationsEnabled}|'
              '${candidate.pushSubscriptionState.pendingTombstoneGeneration}',
        tombstoneRetry.value,
      ],
    );

    useEffect(
      () {
        if (!_ready(session, config, community, memberPubkey) ||
            !buzzPushLifecycleEnabled(
              community: community,
              descriptor: descriptor,
            )) {
          return null;
        }
        final activeCommunity = community!;
        final activeDescriptor = descriptor!;
        final attempt = '${activeCommunity.id}|${config.baseUrl}';
        if (!registrationAttempt.tryBegin(attempt)) return null;
        unawaited(() async {
          try {
            await startBuzzPushRegistrationIfCapable(
              activeDescriptor,
              startRegistration: startBuzzPushRegistration,
            );
          } catch (error, stack) {
            registrationAttempt.failed(
              attempt,
              retry: () {
                if (context.mounted) registrationRetry.value += 1;
              },
            );
            debugPrint('Push registration bootstrap failed: $error');
            debugPrintStack(stackTrace: stack);
          }
        }());
        return null;
      },
      [
        session.status,
        config.baseUrl,
        community?.id,
        memberPubkey,
        descriptor,
        registrationRetry.value,
      ],
    );

    final token = apnsDeviceToken.value;
    useEffect(
      () {
        if (!_ready(session, config, community, memberPubkey) ||
            !buzzPushLifecycleEnabled(
              community: community,
              descriptor: descriptor,
            ) ||
            token == null) {
          return null;
        }
        final activeCommunity = community!;
        final activeDescriptor = descriptor!;
        final state = activeCommunity.pushSubscriptionState;
        if (state.desired.isEmpty) return null;
        final attempt = buzzPushPublicationAttemptKey(
          communityId: activeCommunity.id,
          relayBaseUrl: config.baseUrl,
          token: token,
          descriptor: activeDescriptor,
          subscriptions: state.desired,
        );
        if (!publicationAttempt.tryBegin(attempt)) return null;
        final relay = SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: config.nsec!,
        );
        unawaited(() async {
          try {
            final grant = await _publish(
              ref,
              config,
              activeCommunity,
              memberPubkey!,
              relay,
            );
            final renewInMilliseconds =
                grant.expiresAt * 1000 -
                DateTime.now().millisecondsSinceEpoch -
                const Duration(minutes: 5).inMilliseconds;
            publicationAttempt.retryAfter(
              attempt,
              delay: Duration(
                milliseconds: renewInMilliseconds > 1000
                    ? renewInMilliseconds
                    : 1000,
              ),
              retry: () {
                if (context.mounted) publicationRetry.value += 1;
              },
            );
          } catch (error, stack) {
            publicationAttempt.failed(
              attempt,
              retry: () {
                if (context.mounted) publicationRetry.value += 1;
              },
            );
            debugPrint('Push lease bootstrap failed: $error');
            debugPrintStack(stackTrace: stack);
          }
        }());
        return null;
      },
      [
        session.status,
        config.baseUrl,
        community?.id,
        community?.pushSubscriptionState,
        memberPubkey,
        descriptor,
        token,
        publicationRetry.value,
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

  static Future<BuzzPushEndpointGrant> _publish(
    WidgetRef ref,
    RelayConfig config,
    Community community,
    String memberPubkey,
    SignedEventRelay relay,
  ) async {
    final state = community.pushSubscriptionState;
    final desired = state.desired;
    final descriptor = await fetchBuzzPushLeaseDescriptor(config.baseUrl);
    final grant = await enrollBuzzPush(
      config.wsUrl,
      Env.pushGatewayUrl,
      communitiesForSnapshotRefresh:
          ref.read(communityListProvider).value ?? [community],
    );
    // Relay lease replacement and gateway delegation are independent state
    // machines. Subscription changes advance only the kind-30350 generation;
    // the opaque grant remains reusable until its own authority changes.
    final notifier = ref.read(communityListProvider.notifier);
    await publishBuzzPushLeaseRecoverably(
      reserveGeneration: () =>
          notifier.reservePushLeaseGeneration(community.id),
      publish: (leaseGeneration) => publishBuzzDevPushLeaseThroughRelay(
        grant: grant,
        leaseInstallationId: community.pushLeaseInstallationId,
        leaseGeneration: leaseGeneration,
        descriptor: descriptor,
        nsec: config.nsec!,
        memberPubkey: memberPubkey,
        subscriptions: desired,
        relay: relay,
      ),
      markAccepted: (leaseGeneration) => notifier.markPushLeaseAccepted(
        community.id,
        subscriptions: desired,
        generation: leaseGeneration,
      ),
    );
    return grant;
  }
}

void _runRevocationOutbox(Future<void> Function() operation) {
  unawaited(
    operation().catchError((Object error, StackTrace stackTrace) {
      reportPushLeaseCleanupError(error, stackTrace);
    }),
  );
}
