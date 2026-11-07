import 'hosted_account_config.dart';

/// Trusted build configuration, never supplied by a deep link or relay.
abstract final class HostedEnvironment {
  /// Account UI and startup work are opt-in, independently of the backend URL.
  static bool get enabled => authDomains.isNotEmpty;

  static final _account = HostedAccountConfig(
    accountUrl: const String.fromEnvironment(
      'BUZZ_BUILDERLAB_URL',
      defaultValue: 'https://app.builderlab.xyz',
    ),
    relayHosts: const String.fromEnvironment('BUZZ_HOSTED_RELAY_HOSTS'),
    communityDomain: const String.fromEnvironment(
      'BUZZ_HOSTED_COMMUNITY_DOMAIN',
    ),
    avatarsCdnUrl: const String.fromEnvironment('BUZZ_AVATARS_CDN_URL'),
  );

  static Uri get origin => _account.origin;
  static Uri? get avatarsCdnOrigin => _account.avatarsCdnOrigin;
  static bool get canCreateCommunity =>
      enabled && _account.communityDomain != null;
  static String get communitySuffix =>
      _account.communityDomain ??
      (throw StateError('This build has no hosted community creation domain'));

  static String communityDisplayName(String name, String relayUrl) =>
      _account.communityDisplayName(name, relayUrl);

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

  /// Explicit build configuration, never inferred from relay metadata.
  static Uri? accountOriginFor(Uri relay) => _account.accountOriginFor(relay);

  /// Whether this build can use its account session for this relay.
  static bool supportsRelay(Uri relay) => accountOriginFor(relay) == origin;
}
