import 'package:buzz/shared/me/me_agent_change.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';

class _Store extends InMemorySharedPreferencesStore {
  bool failPreference = false;
  bool failJournal = false;
  _Store() : super.empty();
  @override
  Future<bool> setValue(String valueType, String key, Object value) async {
    if (failJournal && key.endsWith(':pending')) return false;
    if (failPreference &&
        key.startsWith('flutter.me-assistant:') &&
        !key.endsWith(':pending')) {
      return false;
    }
    return super.setValue(valueType, key, value);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late SharedPreferences prefs;
  late _Store store;
  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    store = _Store();
    SharedPreferencesStorePlatform.instance = store;
    prefs = await SharedPreferences.getInstance();
  });
  MeSync service({String relay = 'relay.test', String secret = '1'}) => MeSync(
    RelaySessionNotifier(),
    RelayConfig(
      baseUrl: 'https://$relay',
      nsec: nostr.Nip19.encode(
        prefix: nostr.Nip19Prefix.nsec,
        data: '0' * 63 + secret,
      ),
    ),
    prefs: prefs,
  );
  for (final boundary in ['before add', 'after add', 'preference']) {
    test('restart resumes agent change after $boundary failure', () async {
      var member = false;
      var adds = 0;
      var fail = true;
      final intent = MeAgentChange.create(service(), 'chat', 'private-agent');
      Future<void> ensure() async {
        expect(
          MeAgentChange.pending(service(), 'chat')?.agent,
          'private-agent',
        );
        if (fail && boundary == 'before add') throw StateError('crash');
        if (!member) {
          member = true;
          adds++;
        }
        if (fail && boundary == 'after add') throw StateError('lost ack');
      }

      store.failPreference = boundary == 'preference';
      await expectLater(
        intent.complete(service(), 'chat', ensureMembership: ensure),
        throwsStateError,
      );
      expect(
        prefs.getString(
          '${MeAgentChange.selectionKey(service(), 'chat')}:pending',
        ),
        isNot(contains('private-agent')),
      );
      final restored = MeAgentChange.pending(service(), 'chat')!;
      fail = false;
      store.failPreference = false;
      await restored.complete(service(), 'chat', ensureMembership: ensure);
      expect(adds, 1);
      expect(
        prefs.getString(MeAgentChange.selectionKey(service(), 'chat')),
        'private-agent',
      );
      expect(MeAgentChange.pending(service(), 'chat'), isNull);
    });
  }
  test('failed durable write never grants access', () async {
    store.failJournal = true;
    var accessed = false;
    await expectLater(
      MeAgentChange.create(service(), 'chat', 'agent').complete(
        service(),
        'chat',
        ensureMembership: () async {
          accessed = true;
        },
      ),
      throwsStateError,
    );
    expect(accessed, isFalse);
  });
  test('journal isolates scope and compares exact reviewed record', () async {
    final sync = service();
    final first = MeAgentChange.create(sync, 'chat', 'agent');
    await first.save(sync, 'chat');
    expect(MeAgentChange.pending(service(relay: 'other.test'), 'chat'), isNull);
    expect(MeAgentChange.pending(service(secret: '2'), 'chat'), isNull);
    expect(MeAgentChange.pending(sync, 'other-chat'), isNull);
    await expectLater(
      first.discard(service(relay: 'other.test'), 'chat'),
      throwsStateError,
    );
    await first.discard(sync, 'chat');
    final second = MeAgentChange.create(sync, 'chat', 'agent');
    await second.save(sync, 'chat');
    await expectLater(first.discard(sync, 'chat'), throwsStateError);
    expect(MeAgentChange.pending(sync, 'chat'), isNotNull);
  });
}
