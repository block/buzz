import 'package:clock/clock.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/diagnostics/diagnostic_reporter.dart';
import '../../shared/diagnostics/network_event_reporting.dart';
import '../../shared/http/app_http_client.dart';
import '../../shared/relay/relay.dart';

/// Counts a channel opening: from asking for its newest history to showing
/// it, or to the failure that stopped it. A catch-up after a reconnect is not
/// an open, so callers time only a channel's first load.
void noteChannelOpen(Ref ref, DateTime startedAt, {Object? error}) {
  final host = Uri.tryParse(ref.read(relayConfigProvider).baseUrl)?.host;
  if (host == null || host.isEmpty) return;
  final failure = error == null
      ? null
      : isRelayChannelAccessDenied(error)
      ? NetworkFailure.relayRefused
      : classifyTransportError(error) ?? NetworkFailure.other;
  ref
      .read(diagnosticReporterProvider)
      .noteNetworkEvent(
        NetworkEvent(
          backend: host,
          endpoint: NetworkEndpoint.channelOpen,
          outcome: failure == null
              ? NetworkOutcome.success
              : NetworkOutcome.failure,
          failure: failure,
          duration: clock.now().difference(startedAt),
        ),
      );
}
