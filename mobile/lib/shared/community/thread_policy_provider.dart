import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import 'community_icon_provider.dart';

/// NIP-TP community thread policy.
enum ThreadPolicy {
  /// Each reply stays under the parent it was sent to.
  nested,

  /// Every reply is presented as a direct reply to its thread root.
  flat,
}

/// The thread policy published in [relayUrl]'s NIP-11 document.
///
/// Only the exact string `"flat"` selects [ThreadPolicy.flat]; a missing or
/// unknown value is [ThreadPolicy.nested]. An unreachable relay or malformed
/// document is an error, so callers retry instead of caching a wrong answer.
///
/// Each relay session (re)connection refetches it, so a policy changed by
/// another admin applies after the next reconnect. Readers keep the previous
/// answer while a refetch runs or fails.
final communityThreadPolicyProvider =
    FutureProvider.family<ThreadPolicy, String>((ref, relayUrl) async {
      ref.watch(
        relaySessionProvider.select(
          (state) => state.status == SessionStatus.connected,
        ),
      );
      final cancelled = Completer<void>();
      ref.onDispose(cancelled.complete);
      final document = await fetchRelayInformationDocument(
        ref.read(communityIconHttpClientProvider),
        relayUrl,
        cancelled.future,
      );
      if (document == null) {
        throw Exception('Relay information unavailable for $relayUrl');
      }
      return document['thread_policy'] == 'flat'
          ? ThreadPolicy.flat
          : ThreadPolicy.nested;
    });

/// The active community's thread policy.
///
/// Reads [ThreadPolicy.nested], the NIP-TP default, until the relay answers
/// and while it cannot be reached; a refresh keeps the last answer.
final activeThreadPolicyProvider = Provider.autoDispose<ThreadPolicy>((ref) {
  final relayUrl = ref.watch(relayConfigProvider.select((c) => c.baseUrl));
  return ref.watch(communityThreadPolicyProvider(relayUrl)).value ??
      ThreadPolicy.nested;
});

/// Publishes a kind:9034 command setting the active community's thread policy,
/// then refetches the policy from NIP-11.
///
/// The relay accepts it only from admins and owners; a rejection throws.
final setThreadPolicyProvider =
    Provider<Future<void> Function(ThreadPolicy policy)>((ref) {
      return (policy) async {
        final config = ref.read(relayConfigProvider);
        await SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: config.nsec,
        ).submit(
          kind: EventKind.setThreadPolicy,
          content: '',
          tags: [
            ['thread_policy', policy.name],
          ],
        );
        ref.invalidate(communityThreadPolicyProvider(config.baseUrl));
      };
    });
