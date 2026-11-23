import 'package:buzz/features/search/name_match.dart';
import 'package:flutter_test/flutter_test.dart';

// Expected ranks come from buzz-app `src/features/search/match.ts`
// (`matchRank`) run on the same inputs, so both clients rank a query alike.
void main() {
  test('channel names rank like desktop', () {
    const cases = [
      ('general', 'general', MatchRank.exact),
      ('Design', 'design', MatchRank.exact),
      ('4things', '4things', MatchRank.exact),
      ('general-chat', 'general', MatchRank.prefix),
      ('team-general', 'general', MatchRank.wordStart),
      ('team-4things', '4things', MatchRank.wordStart),
      ('release-notes', 'notes', MatchRank.wordStart),
      ('Ünïcode-ok', 'ok', MatchRank.wordStart),
      ('the4things', '4things', MatchRank.substring),
      ('release-notes', 'ease', MatchRank.substring),
      ('release-notes', 'releasenotes', MatchRank.wordRuns),
      ('release-notes', 'release notes', MatchRank.wordRuns),
      ('buzz-community', 'buzz community', MatchRank.wordRuns),
      ('team-launch-planning', 'tlp', MatchRank.wordRuns),
      ('release-notes', 'reln', MatchRank.wordRuns),
      ('build_and_deploy', 'bd', MatchRank.wordRuns),
      ('dev', 'd e v', MatchRank.wordRuns),
      ('general', 'genral', MatchRank.inOrder),
      ('release-notes', 'budget', null),
      ('abc', 'ca', null),
    ];
    for (final (label, needle, rank) in cases) {
      expect(matchRank(label, needle), rank, reason: '$label / $needle');
    }
  });
}
