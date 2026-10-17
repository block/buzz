/// Trusted build configuration, never supplied by a deep link or relay.
abstract final class HostedEnvironment {
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

  /// Reserved domains cannot fall back to Nostr-only onboarding, even when
  /// this build does not support their account environment.
  static bool isReservedHost(String host) {
    final normalized = host.toLowerCase().replaceFirst(RegExp(r'\.+$'), '');
    return [
      'buzz.xyz',
      'builderlab.xyz',
      'builderlabstaging.xyz',
    ].any((domain) => normalized == domain || normalized.endsWith('.$domain'));
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
