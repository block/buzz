import 'package:flutter/foundation.dart';

import 'builderlab_build.dart';

/// The organization's identity provider (a Builderlab tenant) of an internal
/// build. Trusted build configuration from [BuilderlabBuild], never supplied
/// by a deep link or relay. Organization relays get NIP-FI assertions from
/// this origin's account session.
abstract final class OrganizationEnvironment {
  static bool get enabled =>
      BuilderlabBuild.distribution == BuilderlabDistribution.internal;

  /// The internal build's Builderlab, for example
  /// `https://organization.example.com`.
  static Uri? get origin => enabled ? BuilderlabBuild.origin : null;

  /// Relay domains that trust this tenant. The first is the organization's
  /// community, which the build's sign-in joins.
  static List<String> get relayHosts =>
      enabled ? BuilderlabBuild.relayDomains : const [];

  /// Whether this build attaches its organization session's assertions here.
  static bool protects(Uri relay) => matches(relay, relayHosts);

  /// Only the exact configured hosts, never their subdomains.
  @visibleForTesting
  static bool matches(Uri relay, List<String> hosts) =>
      BuilderlabBuild.matchesRelay(relay, hosts) &&
      hosts.contains(relay.host.toLowerCase());
}
