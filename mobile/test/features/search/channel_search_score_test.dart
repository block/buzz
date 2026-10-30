import 'package:buzz/features/search/channel_search_score.dart';
import 'package:flutter_test/flutter_test.dart';

// Mirrors desktop/src/features/channels/lib/channelSearchScore.test.mjs so
// both clients find the same channels for the same query.
void main() {
  test('channel names match the same queries as desktop', () {
    const matches = [
      ('release-notes', ''),
      ('release-notes', 'notes'),
      ('release-notes', 'not'),
      ('release-notes', 'ease'),
      ('release-notes', 'releasenotes'),
      ('release-notes', 'releasenot'),
      ('release-notes', 'reln'),
      ('build_and_deploy', 'bd'),
      ('v1.2.release', 'vrel'),
      ('general', 'genral'),
      ('engineering', 'engienering'),
      ('4things', '4things'),
      ('team-4things', '4things'),
    ];
    for (final (name, query) in matches) {
      expect(scoreChannelName(name, query), isNotNull, reason: '$name/$query');
    }

    const misses = [
      ('release-notes', 'z'),
      ('release-notes', 'x'),
      ('release-notes', 'budget'),
      ('general', 'gxl'),
      ('general', 'gxxral'),
      ('abc', 'ca'),
    ];
    for (final (name, query) in misses) {
      expect(scoreChannelName(name, query), isNull, reason: '$name/$query');
    }
  });

  test('stronger channel name matches rank first', () {
    // Each name/query pair must score strictly better than the next one.
    const ordered = [
      ('general', 'general'),
      ('general-chat', 'general'),
      ('team-general', 'general'),
    ];
    const fuzzier = [
      ('release-notes', 'release'),
      ('release-notes', 'releasenotes'),
      ('release-notes', 'reln'),
    ];
    for (final chain in [ordered, fuzzier]) {
      for (var index = 1; index < chain.length; index++) {
        final (betterName, betterQuery) = chain[index - 1];
        final (worseName, worseQuery) = chain[index];
        expect(
          scoreChannelName(betterName, betterQuery)!,
          lessThan(scoreChannelName(worseName, worseQuery)!),
        );
      }
    }
    expect(
      scoreChannelName('general', 'gene')!,
      lessThan(scoreChannelName('general', 'genral')!),
    );
  });

  test('descriptions match plain substrings below every name match', () {
    final nameHit = scoreChannelMatch(
      name: 'release-notes',
      description: '',
      lowerQuery: 'release',
    );
    final descriptionHit = scoreChannelMatch(
      name: 'random',
      description: 'Release coordination',
      lowerQuery: 'release',
    );
    expect(nameHit!, lessThan(descriptionHit!));
    expect(
      scoreChannelMatch(
        name: 'random',
        description: 'release notes',
        lowerQuery: 'reln',
      ),
      isNull,
    );
  });
}
