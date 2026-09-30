import 'package:buzz/shared/utils/adaptive_layout.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('uses the persistent workspace for tablet-class windows', () {
    expect(usesTabletWorkspace(width: 1024, height: 768), isTrue);
    expect(usesTabletWorkspace(width: 840, height: 600), isTrue);
  });

  test('keeps wide landscape phones in the compact shell', () {
    expect(usesTabletWorkspace(width: 932, height: 430), isFalse);
  });

  test('falls back when split view is too narrow', () {
    expect(usesTabletWorkspace(width: 700, height: 1024), isFalse);
  });
}
