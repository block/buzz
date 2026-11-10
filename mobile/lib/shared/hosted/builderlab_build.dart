import 'package:flutter/foundation.dart';

/// Who a build is distributed to. See `CONTEXT.md` and
/// `docs/adr/0001-internal-and-public-mobile-builds.md`.
enum BuilderlabDistribution {
  /// An organization's own app: only its Builderlab and its community.
  internal,

  /// The publicly distributed app: only the public Builderlab.
  public,
}

/// Trusted build configuration naming this build's single Builderlab, never
/// supplied by a deep link or relay. With none of the settings present, the
/// build is Nostr-only.
abstract final class BuilderlabBuild {
  static final ({
    BuilderlabDistribution? distribution,
    Uri? origin,
    List<String> relayDomains,
  })
  _config = parse(
    distribution: const String.fromEnvironment('BUZZ_DISTRIBUTION'),
    url: const String.fromEnvironment('BUZZ_BUILDERLAB_URL'),
    relays: const String.fromEnvironment('BUZZ_BUILDERLAB_RELAYS'),
    retired: [
      if (const bool.hasEnvironment('BUZZ_AUTH_DOMAINS')) 'BUZZ_AUTH_DOMAINS',
      if (const bool.hasEnvironment('BUZZ_ORGANIZATION_URL'))
        'BUZZ_ORGANIZATION_URL',
      if (const bool.hasEnvironment('BUZZ_ORGANIZATION_RELAYS'))
        'BUZZ_ORGANIZATION_RELAYS',
    ],
  );

  /// Keys the public Builderlab session's storage in builds without a public
  /// Builderlab, which an internal build clears at launch.
  static final Uri publicStorageOrigin = Uri.parse(
    'https://app.builderlab.xyz',
  );

  /// `BUZZ_DISTRIBUTION`: `internal` or `public`. Null for a Nostr-only build.
  static BuilderlabDistribution? get distribution => _config.distribution;

  /// `BUZZ_BUILDERLAB_URL`: this build's Builderlab HTTPS origin.
  static Uri? get origin => _config.origin;

  /// `BUZZ_BUILDERLAB_RELAYS`: comma-separated DNS domains whose relays use
  /// this build's Builderlab session. A public build also matches their
  /// subdomains; an internal build matches only these exact hosts.
  static List<String> get relayDomains => _config.relayDomains;

  /// Matches a configured domain and its subdomains, never a string suffix.
  static bool matchesHost(String host, List<String> domains) {
    final normalized = host.toLowerCase().replaceFirst(RegExp(r'\.+$'), '');
    return domains.any(
      (domain) => normalized == domain || normalized.endsWith('.$domain'),
    );
  }

  /// Whether [relay] is a plain origin on a configured domain, so this build
  /// may send its Builderlab session's assertions there.
  static bool matchesRelay(Uri relay, List<String> domains) =>
      ['wss', 'https'].contains(relay.scheme) &&
      relay.userInfo.isEmpty &&
      !relay.hasQuery &&
      !relay.hasFragment &&
      (relay.path.isEmpty || relay.path == '/') &&
      (!relay.hasPort || relay.port == 443) &&
      !relay.host.endsWith('.') &&
      matchesHost(relay.host, domains);

  @visibleForTesting
  static ({
    BuilderlabDistribution? distribution,
    Uri? origin,
    List<String> relayDomains,
  })
  parse({
    required String distribution,
    required String url,
    required String relays,
    List<String> retired = const [],
  }) {
    // A pipeline still passing the former settings would otherwise ship a
    // Nostr-only build without its organization restriction.
    if (retired.isNotEmpty) {
      throw StateError(
        '${retired.join(', ')} retired: set BUZZ_DISTRIBUTION, '
        'BUZZ_BUILDERLAB_URL and BUZZ_BUILDERLAB_RELAYS instead',
      );
    }
    final domains = _parseDomains(relays);
    if (distribution.isEmpty && url.isEmpty && domains.isEmpty) {
      return (distribution: null, origin: null, relayDomains: const []);
    }
    if (distribution.isEmpty || url.isEmpty || domains.isEmpty) {
      throw StateError(
        'BUZZ_DISTRIBUTION, BUZZ_BUILDERLAB_URL and BUZZ_BUILDERLAB_RELAYS must be set together',
      );
    }
    final kind = BuilderlabDistribution.values
        .where((value) => value.name == distribution)
        .singleOrNull;
    if (kind == null) {
      throw StateError('BUZZ_DISTRIBUTION must be internal or public');
    }
    // An internal build reaches one organization community and has no
    // switcher, so a second relay could strand its members there.
    if (kind == BuilderlabDistribution.internal && domains.length != 1) {
      throw StateError('An internal build takes exactly one relay host');
    }
    final uri = Uri.parse(url);
    if (uri.scheme != 'https' ||
        uri.host.isEmpty ||
        uri.userInfo.isNotEmpty ||
        uri.hasPort ||
        uri.hasQuery ||
        uri.hasFragment ||
        (uri.path.isNotEmpty && uri.path != '/')) {
      throw StateError('BUZZ_BUILDERLAB_URL must be an HTTPS origin');
    }
    final origin = Uri.parse(uri.origin);
    // Sessions are stored per origin host, so the internal build's cleanup of
    // the public session would delete its own.
    if (kind == BuilderlabDistribution.internal &&
        origin.host == publicStorageOrigin.host) {
      throw StateError(
        'An internal build needs its own Builderlab, not ${origin.host}',
      );
    }
    return (distribution: kind, origin: origin, relayDomains: domains);
  }

  static List<String> _parseDomains(String relays) => List.unmodifiable(
    relays
        .split(',')
        .map(
          (domain) =>
              domain.trim().toLowerCase().replaceFirst(RegExp(r'\.+$'), ''),
        )
        .where((domain) => domain.isNotEmpty)
        .map((domain) {
          if (!RegExp(
            r'^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$',
          ).hasMatch(domain)) {
            throw StateError(
              'BUZZ_BUILDERLAB_RELAYS must contain DNS names, not URLs or wildcards',
            );
          }
          return domain;
        }),
  );
}
