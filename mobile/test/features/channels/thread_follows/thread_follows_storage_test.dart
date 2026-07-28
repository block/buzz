import 'package:buzz/features/channels/thread_follows/thread_follows_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

Future<SharedPreferences> _prefs() async {
  SharedPreferences.setMockInitialValues({});
  return SharedPreferences.getInstance();
}

void main() {
  const pubkey = 'abc123';

  test('round-trips entries per pubkey', () async {
    final storage = ThreadFollowsStorage(await _prefs());
    storage.write(pubkey, const [
      ThreadFollowEntry(rootId: 'root-1', followedAt: 100),
      ThreadFollowEntry(rootId: 'root-2', followedAt: 200),
    ]);

    final entries = storage.read(pubkey);
    expect(entries.map((e) => e.rootId), ['root-1', 'root-2']);
    expect(entries.map((e) => e.followedAt), [100, 200]);
    expect(storage.read('other-pubkey'), isEmpty);
  });

  test('ignores malformed stored payloads', () async {
    SharedPreferences.setMockInitialValues({
      threadFollowsKey(pubkey): '{"not":"a list"}',
    });
    final storage = ThreadFollowsStorage(await SharedPreferences.getInstance());
    expect(storage.read(pubkey), isEmpty);
  });

  test('drops malformed entries but keeps valid ones', () async {
    SharedPreferences.setMockInitialValues({
      threadFollowsKey(pubkey):
          '[{"rootId":"good","followedAt":5},{"rootId":42},"junk"]',
    });
    final storage = ThreadFollowsStorage(await SharedPreferences.getInstance());
    final entries = storage.read(pubkey);
    expect(entries, hasLength(1));
    expect(entries.single.rootId, 'good');
  });

  test('capThreadFollowEntries evicts oldest first', () {
    final entries = [
      for (var i = 0; i < threadFollowsMaxEntries + 10; i++)
        ThreadFollowEntry(rootId: 'root-$i', followedAt: i),
    ];
    final capped = capThreadFollowEntries(entries);
    expect(capped, hasLength(threadFollowsMaxEntries));
    expect(capped.first.rootId, 'root-10');
    expect(capped.last.rootId, 'root-${threadFollowsMaxEntries + 9}');
  });

  test('capThreadFollowEntries keeps small lists untouched', () {
    const entries = [ThreadFollowEntry(rootId: 'a', followedAt: 1)];
    expect(capThreadFollowEntries(entries), same(entries));
  });
}
