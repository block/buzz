import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/relay/client_stamp.dart';
import 'package:uuid/uuid.dart';

void main() {
  String tag(ClientStamp stamp, String name) =>
      stamp.tags.singleWhere((t) => t.first == name)[1];

  test('splits the send time into seconds and a millisecond offset', () {
    final stamp = ClientStamp.now(
      DateTime.fromMillisecondsSinceEpoch(1700000000123),
    );
    expect(stamp.createdAt, 1700000000);
    expect(tag(stamp, 'ms'), '123');
  });

  test('writes the millisecond offset as a canonical 0-999 string', () {
    expect(
      tag(
        ClientStamp.now(DateTime.fromMillisecondsSinceEpoch(1700000000000)),
        'ms',
      ),
      '0',
    );
    expect(
      tag(
        ClientStamp.now(DateTime.fromMillisecondsSinceEpoch(1700000000007)),
        'ms',
      ),
      '7',
    );
    expect(
      tag(
        ClientStamp.now(DateTime.fromMillisecondsSinceEpoch(1700000000999)),
        'ms',
      ),
      '999',
    );
  });

  test('carries a fresh v4 client-id each time', () {
    final at = DateTime.fromMillisecondsSinceEpoch(1700000000123);
    final a = tag(ClientStamp.now(at), 'client-id');
    final b = tag(ClientStamp.now(at), 'client-id');
    expect(Uuid.isValidUUID(fromString: a), isTrue);
    expect(a[14], '4');
    expect(a, isNot(b));
  });
}
