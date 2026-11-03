import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('Unarchive uses restoration instead of crossed-out archiving', () {
    expect(BuzzIcons.archiveRestore.codePoint, 0xfafd);
    expect(BuzzIcons.archiveRestore, isNot(BuzzIcons.archive));
    expect(BuzzIcons.archiveRestore.codePoint, isNot(0xf0ad));
  });

  test('temporary channels use an hourglass distinct from running status', () {
    expect(BuzzIcons.clockFading.codePoint, 0xef93);
    expect(BuzzIcons.clockFading, isNot(BuzzIcons.clock3));
  });
}
