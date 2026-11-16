import 'dart:io';

import 'package:buzz/shared/push/shown_message_store.dart';
import 'package:flutter_test/flutter_test.dart';

String _id(int n) => n.toRadixString(16).padLeft(64, '0');

void main() {
  late Directory directory;
  final now = DateTime.utc(2026, 11, 15, 12);
  ShownMessageStore open() =>
      DirectoryShownMessageStore(directory, now: () => now);

  setUp(() => directory = Directory.systemTemp.createTempSync('shown-'));
  tearDown(() => directory.deleteSync(recursive: true));

  test('of concurrent claims for one message exactly one wins', () async {
    final at = now.subtract(const Duration(minutes: 1));
    final results = await Future.wait([
      for (var i = 0; i < 20; i++) open().claim('community', _id(1), at),
    ]);
    expect(results.where((won) => won), hasLength(1));
  });

  test('history is rejected, reordering and future dates are not', () async {
    final store = open();
    Future<bool> claim(int n, Duration age) =>
        store.claim('community', _id(n), now.subtract(age));

    expect(await claim(1, const Duration(minutes: 1)), isTrue);
    expect(await claim(1, const Duration(minutes: 1)), isFalse);
    // Handled out of order, within the window.
    expect(await claim(2, const Duration(minutes: 10)), isTrue);
    // Older than the window before the newest shown message.
    expect(await claim(3, const Duration(minutes: 30)), isFalse);
    // A message dated in the future is recorded as of now, so it can't hold
    // back later ones.
    expect(await claim(4, const Duration(days: -365)), isTrue);
    expect(await claim(5, const Duration(minutes: 1)), isTrue);
    expect(await store.newestShown('community'), now);
    // Communities don't share a record.
    expect(await store.newestShown('another'), isNull);
    expect(await open().claim('another', _id(1), now), isTrue);
  });

  test('a message pruned from the record is not claimed again', () async {
    final store = open();
    Future<bool> claim(int n, Duration age) =>
        store.claim('community', _id(n), now.subtract(age));

    expect(await claim(1, const Duration(minutes: 20)), isTrue);
    expect(await claim(2, const Duration(minutes: 1)), isTrue);
    // The first marker is now older than the window and has been deleted.
    expect(await claim(1, const Duration(minutes: 20)), isFalse);
  });
}
