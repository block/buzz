import 'package:buzz/shared/drafts/compose_drafts_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _FixedRelayConfigNotifier extends RelayConfigNotifier {
  final RelayConfig _config;
  _FixedRelayConfigNotifier(this._config);

  @override
  RelayConfig build() => _config;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Future<ProviderContainer> containerWithPrefs({
    String relayUrl = 'https://relay-a.example',
    String? pubkey = 'pk_a',
  }) async {
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(RelayConfig(baseUrl: relayUrl)),
        ),
        myPubkeyProvider.overrideWithValue(pubkey),
      ],
    );
    addTearDown(container.dispose);
    return container;
  }

  test('late delivery clears only the submitted draft', () async {
    SharedPreferences.setMockInitialValues({});
    final container = await containerWithPrefs();
    final drafts = container.read(composeDraftsProvider.notifier);
    void save(String text) => drafts.save(key: 'c', channelId: 'c', text: text);
    save('submitted');
    final firstDelivery = drafts.cleanupAfterSend('c');
    save('next draft');
    firstDelivery();
    expect(drafts.textFor('c'), 'next draft');
    drafts.cleanupAfterSend('c')();
    expect(drafts.textFor('c'), isNull);
  });
  test(
    'an invite-joined community keeps its drafts after origin canonicalization',
    () async {
      // Written by a build that stored the invite link's wss:// origin
      // verbatim; RelayConfig now canonicalizes that to https://, so the key
      // the app computes no longer matches the key on disk.
      const legacyKey = 'compose_drafts_v1:wss://relay-a.example:pk_a';
      const canonicalKey = 'compose_drafts_v1:https://relay-a.example:pk_a';
      SharedPreferences.setMockInitialValues({
        legacyKey:
            '[{"key":"ch1","channel_id":"ch1","text":"unsent work",'
            '"updated_at":1700000000}]',
      });

      final container = await containerWithPrefs(
        relayUrl: 'wss://relay-a.example',
      );

      final drafts = container.read(composeDraftsProvider);
      expect(drafts, hasLength(1), reason: 'draft survives the upgrade');
      expect(drafts.single.text, 'unsent work');

      await Future<void>.delayed(Duration.zero);
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getString(canonicalKey), isNotNull);
      expect(prefs.getString(legacyKey), isNull);
    },
  );

  test('label-keyed bindings migrate in one longest-match pass', () {
    final scout = 'a' * 64;
    final marie = 'b' * 64;
    ComposeDraft read(String text, Object? keys) => ComposeDraft.fromJson({
      'key': 'ch1',
      'channel_id': 'ch1',
      'text': text,
      'mention_keys': keys,
      'updated_at': 1,
    })!;
    ComposeDraftMention bound(int start, int end, String key) =>
        ComposeDraftMention(start: start, end: end, pubkey: key, legacy: true);

    for (final (text, keys, mentions) in [
      // The shorter label never takes the longer label's occurrence.
      (
        '@Scout @Scout Marie',
        {'Scout': scout, 'Scout Marie': marie},
        [bound(0, 6, scout), bound(7, 19, marie)],
      ),
      (
        '@Scout Marie and @scout',
        {'Scout Marie': marie, 'Scout': scout},
        [bound(0, 12, marie), bound(17, 23, scout)],
      ),
      // Review of #9699: backticks read as spaces, as the old saver read
      // them, so a label inside code keeps its key. The composer holds the
      // binding dormant while the code is there.
      ('`@Scout` now', {'Scout': scout}, [bound(1, 7, scout)]),
      (
        '`@Scout` and @Scout',
        {'Scout': scout},
        [bound(1, 7, scout), bound(13, 19, scout)],
      ),
      ('`@Scout now`', {'Scout': scout}, [bound(1, 7, scout)]),
      ('`ping @Scout now`', {'Scout': scout}, [bound(6, 12, scout)]),
      ('```\n@Scout\n```', {'Scout': scout}, [bound(4, 10, scout)]),
      ('```@Scout\n```', {'Scout': scout}, [bound(3, 9, scout)]),
      // The longest label still wins inside the backticks.
      (
        '`@Scout Marie`',
        {'Scout': scout, 'Scout Marie': marie},
        [bound(1, 13, marie)],
      ),
      // An unclosed backtick is not code. The label's key is read here all
      // the same; the composer drops it when it takes the draft, since the
      // old send notified nobody after a backtick (exact_mention_tests).
      ('`@Scout hi', {'Scout': scout}, [bound(1, 7, scout)]),
      // A label no longer in the text binds nothing.
      ('hello', {'Scout': scout}, <ComposeDraftMention>[]),
    ]) {
      final draft = read(text, keys);
      expect(draft.mentionsInvalid, isFalse, reason: text);
      expect(draft.mentions, mentions, reason: text);
      // The migrated record is saved as positions, still marked until the
      // composer has read the roster.
      expect(
        ComposeDraft.fromJson(draft.toJson())!.mentions,
        mentions,
        reason: text,
      );
    }

    // Labels that differ only by case never said which key was meant.
    for (final keys in <Object?>[
      {'Scout': scout, 'scout': marie},
      {'Scout': 'not-a-key'},
      {'': scout},
      'Scout',
    ]) {
      final draft = read('@Scout', keys);
      expect(draft.mentionsInvalid, isTrue, reason: '$keys');
      expect(draft.mentions, isEmpty);
    }
  });

  test('composeDraftKey separates channel and thread composers', () {
    expect(composeDraftKey('ch1'), 'ch1');
    expect(composeDraftKey('ch1', threadHeadId: 't1'), 'ch1:t1');
  });

  test('save adds a draft and textFor returns it', () async {
    SharedPreferences.setMockInitialValues({});
    final container = await containerWithPrefs();
    final notifier = container.read(composeDraftsProvider.notifier);

    notifier.save(key: 'ch1', channelId: 'ch1', text: 'hello there');

    final drafts = container.read(composeDraftsProvider);
    expect(drafts, hasLength(1));
    expect(drafts.single.text, 'hello there');
    expect(notifier.textFor('ch1'), 'hello there');
  });

  test('empty text removes the draft', () async {
    SharedPreferences.setMockInitialValues({});
    final container = await containerWithPrefs();
    final notifier = container.read(composeDraftsProvider.notifier);

    notifier.save(key: 'ch1', channelId: 'ch1', text: 'hello');
    notifier.save(key: 'ch1', channelId: 'ch1', text: '   ');

    expect(container.read(composeDraftsProvider), isEmpty);
  });

  test('drafts persist across container restarts', () async {
    SharedPreferences.setMockInitialValues({});
    final first = await containerWithPrefs();
    first
        .read(composeDraftsProvider.notifier)
        .save(key: 'ch1:t1', channelId: 'ch1', threadHeadId: 't1', text: 'wip');

    final second = await containerWithPrefs();
    final restored = second.read(composeDraftsProvider);
    expect(restored, hasLength(1));
    expect(restored.single.channelId, 'ch1');
    expect(restored.single.threadHeadId, 't1');
    expect(restored.single.text, 'wip');
  });

  test('remove deletes by key', () async {
    SharedPreferences.setMockInitialValues({});
    final container = await containerWithPrefs();
    final notifier = container.read(composeDraftsProvider.notifier);

    notifier.save(key: 'ch1', channelId: 'ch1', text: 'hello');
    notifier.remove('ch1');

    expect(container.read(composeDraftsProvider), isEmpty);
  });

  test('malformed persisted json is ignored', () async {
    SharedPreferences.setMockInitialValues({
      'compose_drafts_v1:https://relay-a.example:pk_a': '{not valid',
    });
    final container = await containerWithPrefs();
    expect(container.read(composeDraftsProvider), isEmpty);
  });

  test('drafts are isolated per community', () async {
    SharedPreferences.setMockInitialValues({});
    final communityA = await containerWithPrefs(
      relayUrl: 'https://relay-a.example',
    );
    communityA
        .read(composeDraftsProvider.notifier)
        .save(key: 'ch1', channelId: 'ch1', text: 'secret from A');

    // Same channel id in another community must not see A's draft.
    final communityB = await containerWithPrefs(
      relayUrl: 'https://relay-b.example',
    );
    expect(communityB.read(composeDraftsProvider), isEmpty);
    expect(
      communityB.read(composeDraftsProvider.notifier).textFor('ch1'),
      isNull,
    );

    // A's own store is untouched.
    final communityAAgain = await containerWithPrefs(
      relayUrl: 'https://relay-a.example',
    );
    expect(
      communityAAgain.read(composeDraftsProvider.notifier).textFor('ch1'),
      'secret from A',
    );
  });

  test('drafts are isolated per account pubkey', () async {
    SharedPreferences.setMockInitialValues({});
    final accountA = await containerWithPrefs(pubkey: 'pk_a');
    accountA
        .read(composeDraftsProvider.notifier)
        .save(key: 'ch1', channelId: 'ch1', text: 'account A draft');

    // Same relay, different account: a colliding channel id must not
    // restore the other account's draft.
    final accountB = await containerWithPrefs(pubkey: 'pk_b');
    expect(accountB.read(composeDraftsProvider), isEmpty);
    expect(
      accountB.read(composeDraftsProvider.notifier).textFor('ch1'),
      isNull,
    );
  });

  test('in-place identity change rebuilds onto the new store', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        // Real derivation path: myPubkeyProvider derives from the config's
        // nsec, so an in-place config change simulates an account/community
        // switch without restarting the container.
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(
            const RelayConfig(baseUrl: 'https://relay-a.example'),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);

    // Seed a draft for the initial identity (nsec-less config → 'anon').
    container
        .read(composeDraftsProvider.notifier)
        .save(key: 'ch1', channelId: 'ch1', text: 'first identity draft');
    expect(container.read(composeDraftsProvider), hasLength(1));

    // Switch community in place: the drafts provider must rebuild against
    // the new identity's (empty) store, not keep the old state.
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://relay-b.example');
    expect(container.read(composeDraftsProvider), isEmpty);
    expect(
      container.read(composeDraftsProvider.notifier).textFor('ch1'),
      isNull,
    );

    // Switching back restores the original identity's draft.
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://relay-a.example');
    expect(
      container.read(composeDraftsProvider.notifier).textFor('ch1'),
      'first identity draft',
    );
  });
}
