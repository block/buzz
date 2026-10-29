/// Trusted build configuration, never supplied by a deep link or relay.
abstract final class HostedEnvironment {
  /// Account UI and startup work are opt-in, independently of the backend URL.
  static bool get enabled => authDomains.isNotEmpty;

  static final Uri origin = _origin();
  static const _communityAccounts = {
    'communities.buzz.xyz': 'https://app.builderlab.xyz',
    'communities.public.builderlabstaging.xyz':
        'https://app.builderlabstaging.xyz',
  };

  /// Creation and admission use the same trusted build-specific host mapping.
  static String get communitySuffix => _communityAccounts.entries
      .singleWhere((entry) => Uri.parse(entry.value) == origin)
      .key;

  /// Comma-separated DNS domains requiring Builderlab authentication.
  /// Empty builds keep Nostr-only entry available, including hosted QR pairing.
  static final List<String> authDomains = List.unmodifiable(
    const String.fromEnvironment('BUZZ_AUTH_DOMAINS')
        .split(',')
        .map(
          (domain) =>
              domain.trim().toLowerCase().replaceFirst(RegExp(r'\.+$'), ''),
        )
        .where((domain) => domain.isNotEmpty)
        .map((domain) {
          if (!RegExp(
            r'^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$',
          ).hasMatch(domain)) {
            throw StateError(
              'BUZZ_AUTH_DOMAINS must contain DNS names, not URLs or wildcards',
            );
          }
          return domain;
        }),
  );

  /// Matches a configured domain and its subdomains, never a string suffix.
  static bool requiresAuth(String host) {
    final normalized = host.toLowerCase().replaceFirst(RegExp(r'\.+$'), '');
    return authDomains.any(
      (domain) => normalized == domain || normalized.endsWith('.$domain'),
    );
  }

  /// Explicit community-to-account mapping. Never infer an account endpoint
  /// from arbitrary relay metadata or send this build's session across realms.
  static Uri? accountOriginFor(Uri relay) {
    if (!['wss', 'https'].contains(relay.scheme) ||
        relay.userInfo.isNotEmpty ||
        relay.hasQuery ||
        relay.hasFragment ||
        (relay.path.isNotEmpty && relay.path != '/') ||
        (relay.hasPort && relay.port != 443)) {
      return null;
    }
    final host = relay.host.toLowerCase();
    for (final entry in _communityAccounts.entries) {
      if (host.endsWith('.${entry.key}')) return Uri.parse(entry.value);
    }
    return null;
  }

  /// Whether this build can use its account session for this relay.
  static bool supportsRelay(Uri relay) => accountOriginFor(relay) == origin;

  static Uri _origin() {
    const configured = String.fromEnvironment(
      'BUZZ_BUILDERLAB_URL',
      defaultValue: 'https://app.builderlab.xyz',
    );
    final uri = Uri.parse(configured);
    if (!{
      'https://app.builderlab.xyz',
      'https://app.builderlabstaging.xyz',
    }.contains(configured)) {
      throw StateError('Unsupported hosted account environment');
    }
    return uri;
  }
}
