import 'package:flutter/foundation.dart';

import 'relay_session.dart';

@immutable
class RelayConnectionPresentation {
  final String title;
  final String detail;

  const RelayConnectionPresentation({
    required this.title,
    required this.detail,
  });
}

bool isTailnetRelayUrl(String relayUrl) {
  final uri = Uri.tryParse(relayUrl.trim());
  return uri != null &&
      uri.hasAuthority &&
      uri.host.toLowerCase().endsWith('.ts.net');
}

RelayConnectionPresentation relayConnectionPresentation(
  String relayUrl,
  SessionState state,
) {
  if (state.failureKind == SessionFailureKind.authentication) {
    return const RelayConnectionPresentation(
      title: 'Authentication failed',
      detail: 'Re-pair this community from Command Adviser',
    );
  }

  final isTailnet = isTailnetRelayUrl(relayUrl);
  if (state.failureKind == SessionFailureKind.network) {
    return isTailnet
        ? const RelayConnectionPresentation(
            title: 'Private relay unavailable',
            detail: 'Check Tailscale or VPN',
          )
        : const RelayConnectionPresentation(
            title: 'Relay unavailable',
            detail: 'Check your internet connection',
          );
  }

  return switch (state.status) {
    SessionStatus.connected => RelayConnectionPresentation(
      title: isTailnet ? 'Connected privately' : 'Connected to',
      detail: relayUrl,
    ),
    SessionStatus.connecting ||
    SessionStatus.reconnecting => RelayConnectionPresentation(
      title: isTailnet ? 'Connecting privately' : 'Connecting',
      detail: relayUrl,
    ),
    SessionStatus.disconnected => RelayConnectionPresentation(
      title: 'Disconnected',
      detail: relayUrl,
    ),
  };
}
