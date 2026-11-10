import 'builderlab_build.dart';

/// The public Builderlab account of a public build. Trusted build
/// configuration from [BuilderlabBuild], never supplied by a deep link or
/// relay.
abstract final class HostedEnvironment {
  /// Account UI and startup work run only in a public build.
  static bool get enabled =>
      BuilderlabBuild.distribution == BuilderlabDistribution.public;

  /// The public build's Builderlab. Other builds never contact it; their
  /// value only keys inert local storage.
  static final Uri origin = enabled
      ? BuilderlabBuild.origin!
      : BuilderlabBuild.publicStorageOrigin;

  /// Public CDN serving each published Builderlab agent card's
  /// `<id>/agent.json` and images, for this build's account environment.
  /// Null for an internal build or a Builderlab without a known CDN, which
  /// have no agent cards.
  static Uri? get avatarsCdnOrigin =>
      BuilderlabBuild.distribution == BuilderlabDistribution.internal
      ? null
      : switch (origin.host) {
          'app.builderlab.xyz' => Uri.parse(
            'https://d2z0pbxrxbsiz5.cloudfront.net',
          ),
          'app.builderlabstaging.xyz' => Uri.parse(
            'https://d10hx6rb3fdt3l.cloudfront.net',
          ),
          _ => null,
        };

  /// Shortens hostname fallbacks for display without changing relay identity.
  static String communityDisplayName(String name, String relayUrl) {
    final relay = Uri.tryParse(relayUrl);
    if (relay == null || name != relay.host || !supportsRelay(relay)) {
      return name;
    }
    for (final domain in _relayDomains) {
      final suffix = '.$domain';
      if (name.endsWith(suffix)) {
        return name.substring(0, name.length - suffix.length);
      }
    }
    return name;
  }

  /// Creation and admission place new communities under the first domain.
  static String get communitySuffix =>
      enabled ? BuilderlabBuild.relayDomains.first : _publicCommunities;

  /// Domains whose communities require this build's Builderlab account.
  /// Empty unless [enabled]; such builds keep Nostr-only entry available,
  /// including hosted QR pairing.
  static List<String> get authDomains =>
      enabled ? BuilderlabBuild.relayDomains : const [];

  /// Matches a configured domain and its subdomains, never a string suffix.
  static bool requiresAuth(String host) =>
      BuilderlabBuild.matchesHost(host, authDomains);

  /// Whether this build can use its account session for this relay.
  static bool supportsRelay(Uri relay) =>
      BuilderlabBuild.matchesRelay(relay, _relayDomains);

  /// The public Builderlab's communities, which a build without Builderlab
  /// still recognizes so a later public build finds them unchanged.
  static const _publicCommunities = 'communities.buzz.xyz';

  /// An internal build never treats a relay as a public community.
  static List<String> get _relayDomains =>
      switch (BuilderlabBuild.distribution) {
        BuilderlabDistribution.public => BuilderlabBuild.relayDomains,
        BuilderlabDistribution.internal => const [],
        null => const [_publicCommunities],
      };
}
