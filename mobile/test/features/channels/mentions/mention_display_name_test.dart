import 'package:buzz/features/channels/mentions/mention_display_name.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('appends owner labels to colliding agents', () {
    expect(
      formatDisambiguatedMentionDisplayName(
        displayName: 'Bumble',
        hasNameCollision: true,
        isAgent: true,
        ownerLabel: 'sergior',
      ),
      'Bumble (sergior)',
    );
    expect(
      formatDisambiguatedMentionDisplayName(
        displayName: 'Bumble',
        hasNameCollision: true,
        isAgent: true,
        ownerLabel: 'you',
      ),
      'Bumble (you)',
    );
  });

  test('leaves humans and unique agents unchanged', () {
    expect(
      formatDisambiguatedMentionDisplayName(
        displayName: 'Bumble',
        hasNameCollision: true,
        isAgent: false,
        ownerLabel: null,
      ),
      'Bumble',
    );
    expect(
      formatDisambiguatedMentionDisplayName(
        displayName: 'Bumble',
        hasNameCollision: false,
        isAgent: true,
        ownerLabel: 'sergior',
      ),
      'Bumble',
    );
  });

  test('detects visible display-name collisions case-insensitively', () {
    final counts = countVisibleMentionDisplayNames([
      'Bumble',
      ' bUmBlE ',
      'Fizz',
    ]);

    expect(hasVisibleMentionDisplayNameCollision('bumble', counts), isTrue);
    expect(hasVisibleMentionDisplayNameCollision('Fizz', counts), isFalse);
  });
}
