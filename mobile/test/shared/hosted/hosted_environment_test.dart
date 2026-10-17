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

  test('reserved unsupported hosts never become self-hosted', () {
    for (final host in [
      'buzz.xyz',
      'app.builderlab.xyz',
      'TEAM.builderlabstaging.xyz',
      'team.communities.public.builderlabstaging.xyz.',
    ]) {
      expect(HostedEnvironment.isReservedHost(host), isTrue);
    }
    expect(
      HostedEnvironment.isReservedHost('builderlabstaging.xyz.attacker.test'),
      isFalse,
    );
  });
}
