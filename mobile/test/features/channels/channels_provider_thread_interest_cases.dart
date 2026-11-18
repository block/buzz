part of 'channels_provider_test.dart';

class _CountingPrefsStore extends InMemorySharedPreferencesStore {
  _CountingPrefsStore() : super.empty();

  final writesByKey = <String, int>{};

  @override
  Future<bool> setValue(String valueType, String key, Object value) {
    writesByKey.update(key, (n) => n + 1, ifAbsent: () => 1);
    return super.setValue(valueType, key, value);
  }
}

void _threadInterestCases() {
  test('unread catch-up saves own thread interest once per batch', () async {
    const self = 'me';
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    String hexId(String prefix, int i) =>
        '$prefix${i.toRadixString(16).padLeft(63, '0')}';
    final replyRoots = [for (var i = 0; i < 40; i++) hexId('a', i)];
    final ownRoots = [for (var i = 0; i < 10; i++) hexId('b', i)];
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, self)],
      metadata: [_meta(id: _channelA, name: 'general')],
      recentMessages: [
        for (final (i, root) in replyRoots.indexed)
          NostrEvent(
            id: hexId('c', i),
            pubkey: self,
            createdAt: now - 600 + i,
            kind: 9,
            tags: [
              ['h', _channelA],
              ['e', root, '', 'root'],
              ['e', root, '', 'reply'],
            ],
            content: 'reply $i',
            sig: '',
          ),
        for (final (i, root) in ownRoots.indexed)
          NostrEvent(
            id: root,
            pubkey: self,
            createdAt: now - 300 + i,
            kind: 9,
            tags: const [
              ['h', _channelA],
            ],
            content: 'root $i',
            sig: '',
          ),
      ],
    );
    final store = _CountingPrefsStore();
    SharedPreferencesStorePlatform.instance = store;
    SharedPreferences.resetStatic();
    addTearDown(() {
      SharedPreferences.resetStatic();
      SharedPreferences.setMockInitialValues({});
    });
    final prefs = await SharedPreferences.getInstance();
    final container = _buildContainer(
      session: session,
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await _settle();

    const participated = 'buzz-thread-participation.v1:me';
    const authored = 'buzz-thread-authored.v1:me';
    Set<String> saved(String key) => {
      ...(jsonDecode(prefs.getString(key)!) as List).cast<String>(),
    };
    expect(saved(participated), replyRoots.toSet());
    expect(saved(authored), ownRoots.toSet());
    expect(store.writesByKey['flutter.$participated'], 1);
    expect(store.writesByKey['flutter.$authored'], 1);
  });
}
