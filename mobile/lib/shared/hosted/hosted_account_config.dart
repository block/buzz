/// Trusted account routing supplied by the distributor at build time.
/// Exact relay hosts do not authorize their subdomains. Community domains
/// authorize only subdomains and also define where new communities are created.
class HostedAccountConfig {
  HostedAccountConfig({
    required String accountUrl,
    String relayHosts = '',
    String communityDomain = '',
    String avatarsCdnUrl = '',
  }) : origin = _httpsOrigin(accountUrl),
       relayHosts = List.unmodifiable(
         relayHosts
             .split(',')
             .where((host) => host.trim().isNotEmpty)
             .map(_domain),
       ),
       communityDomain = communityDomain.isEmpty
           ? _publicDomains[accountUrl]
           : _domain(communityDomain),
       avatarsCdnOrigin = avatarsCdnUrl.isEmpty
           ? _publicCdns[accountUrl]
           : _httpsOrigin(avatarsCdnUrl) {
    if (this.relayHosts.isEmpty && this.communityDomain == null) {
      throw StateError(
        'A custom account requires BUZZ_HOSTED_RELAY_HOSTS or '
        'BUZZ_HOSTED_COMMUNITY_DOMAIN',
      );
    }
  }

  final Uri origin;
  final List<String> relayHosts;
  final String? communityDomain;
  final Uri? avatarsCdnOrigin;

  static const _publicDomains = {
    'https://app.builderlab.xyz': 'communities.buzz.xyz',
    'https://app.builderlabstaging.xyz':
        'communities.public.builderlabstaging.xyz',
  };
  static final _publicCdns = {
    'https://app.builderlab.xyz': Uri.parse(
      'https://d2z0pbxrxbsiz5.cloudfront.net',
    ),
    'https://app.builderlabstaging.xyz': Uri.parse(
      'https://d10hx6rb3fdt3l.cloudfront.net',
    ),
  };

  Uri? accountOriginFor(Uri relay) {
    if (!['wss', 'https'].contains(relay.scheme) ||
        relay.userInfo.isNotEmpty ||
        relay.hasQuery ||
        relay.hasFragment ||
        (relay.path.isNotEmpty && relay.path != '/') ||
        (relay.hasPort && relay.port != 443)) {
      return null;
    }
    final host = relay.host.toLowerCase();
    if (relayHosts.contains(host) ||
        (communityDomain != null && host.endsWith('.$communityDomain'))) {
      return origin;
    }
    // Retain recognition of the other public realm, without authorizing this
    // build's account session there (supportsRelay compares account origins).
    for (final entry in _publicDomains.entries) {
      if (host.endsWith('.${entry.value}')) return Uri.parse(entry.key);
    }
    return null;
  }

  String communityDisplayName(String name, String relayUrl) {
    final relay = Uri.tryParse(relayUrl);
    if (relay == null ||
        name != relay.host ||
        accountOriginFor(relay) == null) {
      return name;
    }
    for (final domain in [?communityDomain, ..._publicDomains.values]) {
      final suffix = '.$domain';
      if (name.endsWith(suffix)) {
        return name.substring(0, name.length - suffix.length);
      }
    }
    return name;
  }

  static String _domain(String value) {
    final domain = value.trim().toLowerCase();
    if (domain.length > 253 ||
        !domain
            .split('.')
            .every(
              (label) => RegExp(
                r'^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$',
              ).hasMatch(label),
            )) {
      throw StateError(
        'Hosted relay hosts and community domains must be DNS names',
      );
    }
    return domain;
  }

  static Uri _httpsOrigin(String value) {
    final uri = Uri.tryParse(value);
    if (uri == null ||
        uri.scheme != 'https' ||
        uri.host.isEmpty ||
        uri.userInfo.isNotEmpty ||
        uri.hasQuery ||
        uri.hasFragment ||
        uri.path.isNotEmpty ||
        (uri.hasPort && uri.port != 443)) {
      throw StateError(
        'Hosted account and CDN URLs must be HTTPS origins without a path',
      );
    }
    _domain(uri.host);
    return Uri.parse(uri.origin);
  }
}
