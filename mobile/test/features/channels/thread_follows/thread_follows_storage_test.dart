import 'package:buzz/features/channels/thread_follows/thread_follows_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/thread_ids.dart';

Future<SharedPreferences> _prefs() async {
  SharedPreferences.setMockInitialValues({});
  return SharedPreferences.getInstance();
}

void main() {
  const pubkey = 'abc123';
  const relayUrl = 'https://relay-a.example';

  test('round-trips entries per pubkey', () async {
    final storage = ThreadFollowsStorage(await _prefs());
    storage.write(pubkey, relayUrl, const [
      ThreadFollowEntry(rootId: threadIdRootDash1, followedAt: 100),
      ThreadFollowEntry(rootId: threadIdRootDash2, followedAt: 200),
    ]);

    final entries = storage.read(pubkey, relayUrl);
    expect(entries.map((e) => e.rootId), [
      threadIdRootDash1,
      threadIdRootDash2,
    ]);
    expect(entries.map((e) => e.followedAt), [100, 200]);
    expect(storage.read('other-pubkey', relayUrl), isEmpty);
  });

  test('every community starts from the shared entry', () async {
    SharedPreferences.setMockInitialValues({
      legacyThreadFollowsKey(pubkey):
          '[{"rootId":"root-a","followedAt":5},'
          '{"rootId":"root-b","followedAt":6}]',
    });
    final storage = ThreadFollowsStorage(await SharedPreferences.getInstance());
    storage.write(pubkey, relayUrl, const []);
    expect(storage.read(pubkey, relayUrl), isEmpty);
    expect(
      storage.read(pubkey, 'https://relay-b.example').map((e) => e.rootId),
      ['root-a', 'root-b'],
    );
  });

  test('ignores malformed stored payloads', () async {
    SharedPreferences.setMockInitialValues({
      threadFollowsKey(pubkey, relayUrl): '{"not":"a list"}',
    });
    final storage = ThreadFollowsStorage(await SharedPreferences.getInstance());
    expect(storage.read(pubkey, relayUrl), isEmpty);
  });

  test('drops malformed entries but keeps valid ones', () async {
    SharedPreferences.setMockInitialValues({
      threadFollowsKey(pubkey, relayUrl):
          '[{"rootId":"good","followedAt":5},{"rootId":42},"junk"]',
    });
    final storage = ThreadFollowsStorage(await SharedPreferences.getInstance());
    final entries = storage.read(pubkey, relayUrl);
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
    const entries = [ThreadFollowEntry(rootId: threadIdA, followedAt: 1)];
    expect(capThreadFollowEntries(entries), same(entries));
  });
}
