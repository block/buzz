import 'package:buzz/shared/hosted/builderlab_build.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const relays = 'communities.example.org';

  test('distribution, origin and relays are configured together', () {
    final none = BuilderlabBuild.parse(distribution: '', url: '', relays: '');
    expect(none.distribution, isNull);
    expect(none.origin, isNull);
    expect(none.relayDomains, isEmpty);

    final internal = BuilderlabBuild.parse(
      distribution: 'internal',
      url: 'https://id.example.org/',
      relays: ' Buzz.Example.org ,other.example.org.',
    );
    expect(internal.distribution, BuilderlabDistribution.internal);
    expect(internal.origin, Uri.parse('https://id.example.org'));
    expect(internal.relayDomains, ['buzz.example.org', 'other.example.org']);

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
