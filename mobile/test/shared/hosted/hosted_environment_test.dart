import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
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
