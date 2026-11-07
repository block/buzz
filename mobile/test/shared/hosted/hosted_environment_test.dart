import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_account_config.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'custom build confines account access to explicit hosts and domains',
    () {
      final config = HostedAccountConfig(
        accountUrl: 'https://accounts.example.com',
        relayHosts: 'relay.example.com,other.example.com',
      );
      expect(config.communityDomain, isNull);
      expect(config.avatarsCdnOrigin, isNull);
      for (final host in ['relay.example.com', 'other.example.com']) {
        expect(
          config.accountOriginFor(Uri.parse('wss://$host')),
          config.origin,
        );
      }
      for (final relay in [
        'wss://child.relay.example.com',
        'wss://relay.example.com.attacker.test',
        'wss://notrelay.example.com',
        'wss://relay.example.com.',
        'ws://relay.example.com',
        'wss://relay.example.com:8443',
        'wss://user@relay.example.com',
        'wss://relay.example.com/path',
        'wss://relay.example.com?account=other',
        'wss://relay.example.com#fragment',
      ]) {
        expect(
          config.accountOriginFor(Uri.parse(relay)),
          isNull,
          reason: relay,
        );
      }
      expect(
        config.accountOriginFor(Uri.parse('wss://team.communities.buzz.xyz')),
        isNot(config.origin),
      );
      final communities = HostedAccountConfig(
        accountUrl: 'https://accounts.example.com',
        communityDomain: 'communities.example.com',
        avatarsCdnUrl: 'https://cards.example.com',
      );
      expect(communities.communityDomain, 'communities.example.com');
      expect(
        communities.avatarsCdnOrigin,
        Uri.parse('https://cards.example.com'),
      );
      expect(
        communities.accountOriginFor(
          Uri.parse('wss://team.communities.example.com'),
        ),
        communities.origin,
      );
      expect(
        communities.accountOriginFor(
          Uri.parse('wss://communities.example.com'),
        ),
        isNull,
      );
      expect(
        communities.communityDisplayName(
          'team.communities.example.com',
          'wss://team.communities.example.com',
        ),
        'team',
      );
    },
  );

  test(
    'invalid build inputs fail closed and public defaults remain available',
    () {
      for (final url in [
        '',
        'http://accounts.example.com',
        'https://user@accounts.example.com',
        'https://accounts.example.com/path',
        'https://accounts.example.com?x=1',
        'https://accounts.example.com#x',
        'https://accounts.example.com:8443',
      ]) {
        expect(
          () => HostedAccountConfig(
            accountUrl: url,
            relayHosts: 'relay.example.com',
          ),
          throwsStateError,
          reason: url,
        );
      }
      for (final domain in [
        '*.example.com',
        'https://example.com',
        '.example.com',
        'example.com.',
        'example..com',
        '-example.com',
        'example.com/path',
      ]) {
        expect(
          () => HostedAccountConfig(
            accountUrl: 'https://accounts.example.com',
            relayHosts: domain,
          ),
          throwsStateError,
          reason: domain,
        );
        expect(
          () => HostedAccountConfig(
            accountUrl: 'https://accounts.example.com',
            communityDomain: domain,
          ),
          throwsStateError,
          reason: domain,
        );
      }
      expect(
        () => HostedAccountConfig(accountUrl: 'https://accounts.example.com'),
        throwsStateError,
      );
      expect(
        () => HostedAccountConfig(
          accountUrl: 'https://app.builderlab.xyz',
          avatarsCdnUrl: 'http://cards.example.com',
        ),
        throwsStateError,
      );
      for (final (url, domain, cdn) in [
        (
          'https://app.builderlab.xyz',
          'communities.buzz.xyz',
          'd2z0pbxrxbsiz5.cloudfront.net',
        ),
        (
          'https://app.builderlabstaging.xyz',
          'communities.public.builderlabstaging.xyz',
          'd10hx6rb3fdt3l.cloudfront.net',
        ),
      ]) {
        final config = HostedAccountConfig(accountUrl: url);
        expect(config.communityDomain, domain);
        expect(config.avatarsCdnOrigin?.host, cdn);
        expect(
          config.accountOriginFor(Uri.parse('wss://team.$domain')),
          config.origin,
        );
      }
    },
  );

  test(
    'build settings reach the production environment without enabling login',
    () {
      const hosts = String.fromEnvironment('BUZZ_HOSTED_RELAY_HOSTS');
      if (hosts.isNotEmpty) {
        expect(
          HostedEnvironment.origin.toString(),
          const String.fromEnvironment('BUZZ_BUILDERLAB_URL'),
        );
        expect(
          HostedEnvironment.supportsRelay(
            Uri.parse('wss://${hosts.split(',').first}'),
          ),
          isTrue,
        );
        const domain = String.fromEnvironment('BUZZ_HOSTED_COMMUNITY_DOMAIN');
        const cdn = String.fromEnvironment('BUZZ_AVATARS_CDN_URL');
        expect(
          HostedEnvironment.canCreateCommunity,
          HostedEnvironment.enabled && domain.isNotEmpty,
        );
        expect(
          HostedEnvironment.avatarsCdnOrigin,
          cdn.isEmpty ? isNull : Uri.parse(cdn),
        );
      }
      if (HostedEnvironment.authDomains.isEmpty) {
        expect(HostedEnvironment.enabled, isFalse);
        expect(HostedEnvironment.canCreateCommunity, isFalse);
      }
    },
  );

  test('community realms map only to their trusted account origin', () {
    expect(
      HostedEnvironment.accountOriginFor(
        Uri.parse('wss://team.communities.buzz.xyz'),
      ),
      Uri.parse('https://app.builderlab.xyz'),
    );
    expect(
      HostedEnvironment.accountOriginFor(
        Uri.parse('wss://team.communities.public.builderlabstaging.xyz'),
      ),
      Uri.parse('https://app.builderlabstaging.xyz'),
    );
    for (final value in [
      'wss://team.communities.public.builderlabstaging.xyz.attacker.test',
      'wss://communities.public.builderlabstaging.xyz',
      'wss://team.communities.public.builderlabstaging.xyz.',
      'wss://team.communities.public.builderlabstaging.xyz:8443',
      'ws://team.communities.public.builderlabstaging.xyz',
      'wss://user@team.communities.public.builderlabstaging.xyz',
      'wss://team.communities.public.builderlabstaging.xyz/path',
      'wss://team.communities.public.builderlabstaging.xyz?account=other',
      'wss://team.communities.public.builderlabstaging.xyz#fragment',
      'wss://team.builderlabstaging.xyz',
    ]) {
      expect(
        HostedEnvironment.accountOriginFor(Uri.parse(value)),
        isNull,
        reason: value,
      );
    }
  });

  test('authentication follows only configured DNS domain boundaries', () {
    expect(HostedEnvironment.enabled, HostedEnvironment.authDomains.isNotEmpty);
    for (final domain in HostedEnvironment.authDomains) {
      for (final host in [
        domain,
        'team.$domain',
        'TEAM.${domain.toUpperCase()}.',
      ]) {
        expect(HostedEnvironment.requiresAuth(host), isTrue, reason: host);
      }
      expect(HostedEnvironment.requiresAuth('$domain.attacker.test'), isFalse);
      expect(HostedEnvironment.requiresAuth('not$domain'), isFalse);
    }
    if (HostedEnvironment.authDomains.isEmpty) {
      for (final host in [
        'buzz.xyz',
        'team.communities.buzz.xyz',
        'team.communities.public.builderlabstaging.xyz',
      ]) {
        expect(HostedEnvironment.requiresAuth(host), isFalse);
      }
    }
  });
}
