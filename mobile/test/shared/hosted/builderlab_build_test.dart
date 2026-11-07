import 'package:buzz/shared/hosted/builderlab_build.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const relays = 'communities.example.org';

  test('distribution, origin and relays are configured together', () {
    final none = BuilderlabBuild.parse(distribution: '', url: '', relays: '');
    expect(none.distribution, isNull);
    expect(none.origin, isNull);
    expect(none.relayDomains, isEmpty);

    final public = BuilderlabBuild.parse(
      distribution: 'public',
      url: 'https://id.example.org/',
      relays: ' Buzz.Example.org ,other.example.org.',
    );
    expect(public.distribution, BuilderlabDistribution.public);
    expect(public.origin, Uri.parse('https://id.example.org'));
    expect(public.relayDomains, ['buzz.example.org', 'other.example.org']);

    final internal = BuilderlabBuild.parse(
      distribution: 'internal',
      url: 'https://id.example.org',
      relays: 'Buzz.Example.org.',
    );
    expect(internal.distribution, BuilderlabDistribution.internal);
    expect(internal.relayDomains, ['buzz.example.org']);

    for (final (distribution, url, relays) in [
      ('public', 'https://id.example.org', ''),
      ('public', '', relays),
      ('', 'https://id.example.org', relays),
      ('enterprise', 'https://id.example.org', relays),
      ('public', 'http://id.example.org', relays),
      ('public', 'https://id.example.org/api', relays),
      ('public', 'https://id.example.org:8443', relays),
      ('public', 'https://id.example.org?x=1', relays),
      ('public', 'https://id.example.org', 'wss://$relays'),
      ('public', 'https://id.example.org', '*.example.org'),
      ('public', 'https://id.example.org', 'localhost'),
      ('internal', 'https://id.example.org', 'buzz.example.org,b.example.org'),
      ('internal', 'https://app.builderlab.xyz', 'buzz.example.org'),
    ]) {
      expect(
        () => BuilderlabBuild.parse(
          distribution: distribution,
          url: url,
          relays: relays,
        ),
        throwsStateError,
        reason: '$distribution $url $relays',
      );
    }
    expect(
      () => BuilderlabBuild.parse(
        distribution: '',
        url: '',
        relays: '',
        retired: ['BUZZ_ORGANIZATION_URL'],
      ),
      throwsStateError,
    );
  });

  test('relays match configured domains and subdomains, never suffixes', () {
    bool matches(String url) =>
        BuilderlabBuild.matchesRelay(Uri.parse(url), [relays]);
    for (final url in [
      'wss://$relays',
      'wss://team.$relays',
      'wss://TEAM.${relays.toUpperCase()}',
      'https://team.$relays/',
      'wss://team.$relays:443',
    ]) {
      expect(matches(url), isTrue, reason: url);
    }
    for (final url in [
      'wss://team.$relays.attacker.test',
      'wss://team.$relays.',
      'wss://not$relays',
      'wss://team.$relays:8443',
      'ws://team.$relays',
      'wss://user@team.$relays',
      'wss://team.$relays/path',
      'wss://team.$relays?account=other',
      'wss://team.$relays#fragment',
    ]) {
      expect(matches(url), isFalse, reason: url);
    }
  });
}
