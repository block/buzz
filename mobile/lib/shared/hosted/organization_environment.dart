import 'package:flutter/foundation.dart';

import 'hosted_environment.dart';

/// Trusted build configuration for an organization's identity provider (a
/// Builderlab tenant), never supplied by a deep link or relay. Organization
/// relays get NIP-FI assertions from this origin's account session, separately
/// from hosted communities' account.
abstract final class OrganizationEnvironment {
  static bool get enabled => origin != null;

  /// `BUZZ_ORGANIZATION_URL`: the tenant's HTTPS origin, for example
  /// `https://block.builderlab.xyz`.
  static final Uri? origin = parseOrigin(
    const String.fromEnvironment('BUZZ_ORGANIZATION_URL'),
    const String.fromEnvironment('BUZZ_ORGANIZATION_RELAYS'),
  );

  /// `BUZZ_ORGANIZATION_RELAYS`: comma-separated exact relay hosts that trust
  /// this tenant. Subdomains and other ports never match.
  static final List<String> relayHosts = parseRelayHosts(
    const String.fromEnvironment('BUZZ_ORGANIZATION_RELAYS'),
  );

  /// Whether this build attaches its organization session's assertions here.
  static bool protects(Uri relay) => enabled && matches(relay, relayHosts);

  @visibleForTesting
  static bool matches(Uri relay, List<String> hosts) =>
      ['wss', 'https'].contains(relay.scheme) &&
      relay.userInfo.isEmpty &&
      !relay.hasQuery &&
      !relay.hasFragment &&
      (relay.path.isEmpty || relay.path == '/') &&
      (!relay.hasPort || relay.port == 443) &&
      hosts.contains(relay.host.toLowerCase());

  @visibleForTesting
  static Uri? parseOrigin(String url, String relays) {
    if (url.isEmpty != relays.trim().isEmpty) {
      throw StateError(
        'BUZZ_ORGANIZATION_URL and BUZZ_ORGANIZATION_RELAYS must be set together',
      );
    }
    if (url.isEmpty) return null;
    final uri = Uri.parse(url);
    if (uri.scheme != 'https' ||
        uri.host.isEmpty ||
        uri.userInfo.isNotEmpty ||
        uri.hasPort ||
        uri.hasQuery ||
        uri.hasFragment ||
        (uri.path.isNotEmpty && uri.path != '/')) {
      throw StateError('BUZZ_ORGANIZATION_URL must be an HTTPS origin');
    }
    final origin = Uri.parse(uri.origin);
    // Sessions and browser attempts are stored per origin host.
    if (origin.host == HostedEnvironment.origin.host) {
      throw StateError(
        'BUZZ_ORGANIZATION_URL must differ from the hosted account origin',
      );
    }
    return origin;
  }

  @visibleForTesting
  static List<String> parseRelayHosts(String relays) => List.unmodifiable(
    relays
        .split(',')
        .map((host) => host.trim().toLowerCase())
        .where((host) => host.isNotEmpty)
        .map((host) {
          if (!RegExp(
            r'^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$',
          ).hasMatch(host)) {
            throw StateError(
              'BUZZ_ORGANIZATION_RELAYS must contain DNS names, not URLs or wildcards',
            );
          }
          // Never send one realm's session to another realm's relay.
          if (HostedEnvironment.accountOriginFor(Uri.parse('wss://$host')) !=
              null) {
            throw StateError(
              'BUZZ_ORGANIZATION_RELAYS must not include hosted communities',
            );
          }
          return host;
        }),
  );
}
