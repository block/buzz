import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import '../diagnostics/diagnostic_reporter.dart';
import '../relay/relay_provider.dart';
import '../relay/relay_session.dart';
import 'dev_push_lease.dart';

typedef BuzzPushDescriptorFetcher =
    Future<BuzzPushLeaseDescriptor> Function(String relayBaseUrl);

final buzzPushDescriptorFetcherProvider = Provider<BuzzPushDescriptorFetcher>(
  (ref) => fetchBuzzPushLeaseDescriptor,
);

/// The fully validated push capability advertised by the current relay.
///
/// An unconfigured artifact has no capability and never starts discovery.
/// Discovery fails closed. An absent, malformed, or unreachable NIP-11 push
/// descriptor is represented as no capability, so no notification permission,
/// APNs registration, gateway enrollment, or relay lease can begin.
final currentRelayPushDescriptorProvider =
    FutureProvider.autoDispose<BuzzPushLeaseDescriptor?>((ref) async {
      if (!Env.pushGatewayConfigured) return null;
      final sessionStatus = ref.watch(
        relaySessionProvider.select((session) => session.status),
      );
      final config = ref.watch(relayConfigProvider);
      // Preference and lease bookkeeping do not change relay capability.
      // Watching the whole community rediscovered NIP-11 after every toggle,
      // disabling the off switch when that redundant request could not finish.
      final communityId = ref.watch(
        activeCommunityProvider.select((value) => value.value?.id),
      );
      final memberPubkey = ref.watch(myPubkeyProvider);
      if (sessionStatus != SessionStatus.connected ||
          communityId == null ||
          config.nsec == null ||
          config.nsec!.isEmpty ||
          memberPubkey == null ||
          memberPubkey.isEmpty) {
        return null;
      }

      // A fetch can finish after a community switch has begun, when reports
      // already name the next member. Its report would be misattributed.
      var current = true;
      final unregister = ref.read(communityTransitionProvider).register(
        () async {
          current = false;
        },
      );
      ref.onDispose(() {
        current = false;
        unregister();
      });
      return discoverBuzzPushRelayCapability(
        config.baseUrl,
        fetchDescriptor: ref.read(buzzPushDescriptorFetcherProvider),
        diagnostics: ref.read(diagnosticReporterProvider),
        isCurrent: () => current,
      );
    });

bool _alwaysCurrent() => true;

Future<BuzzPushLeaseDescriptor?> discoverBuzzPushRelayCapability(
  String relayBaseUrl, {
  required BuzzPushDescriptorFetcher fetchDescriptor,
  DiagnosticReporter diagnostics = const NoopDiagnosticReporter(),
  bool Function() isCurrent = _alwaysCurrent,
}) async {
  try {
    return await fetchDescriptor(relayBaseUrl);
  } catch (error, stackTrace) {
    // A relay without Buzz push, or one that can't be reached, is expected; a
    // descriptor that can't be read is not.
    if (error is FormatException &&
        error is! BuzzPushNotAdvertised &&
        isCurrent()) {
      diagnostics.reportError(
        error,
        stackTrace,
        operation: DiagnosticOperation.pushGatewayResponse,
      );
    }
    debugPrint('Current relay does not advertise valid push: $error');
    debugPrintStack(stackTrace: stackTrace);
    return null;
  }
}
