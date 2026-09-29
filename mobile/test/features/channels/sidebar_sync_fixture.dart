import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

/// A scripted relay behind the real [RelaySessionNotifier], so REQ/EOSE,
/// CLOSED classification, event batching and OK all run production code.
/// Retains replaceable heads the way the relay does: `created_at DESC, id ASC`.
class SidebarRelay {
  SidebarRelay() {
    session.debugAttachSocketForTest(_ScriptedSocket(_onSend));
  }

  final keys = nostr.Keys.generate();
  final session = ConnectedRelaySession();
  late final signer = SignedEventRelay(session: session, nsec: keys.nsec);
  final stored = <NostrEvent>[];
  final published = <NostrEvent>[];
  final reqs = <List<dynamic>>[];
  final _liveSubIds = <String, String>{};
  int historyFailures = 0;
  Completer<void>? holdHistory;
  Completer<void>? holdOk;
  String? rejectLive;
  bool withholdEose = false;

  String get pubkey => keys.public;

  NostrEvent event(String dTag, Object payload, int createdAt, {String? id}) =>
      NostrEvent(
        id: id ?? 'e$createdAt'.padRight(64, 'f'),
        pubkey: pubkey,
        createdAt: createdAt,
        kind: EventKind.readState,
        tags: [
          ['d', dTag],
        ],
        content: nip44Encrypt(_key, jsonEncode(payload)),
        sig: 'sig',
      );

  Object? decrypt(NostrEvent event) =>
      jsonDecode(nip44Decrypt(_key, event.content));

  late final _key = getConversationKey(
    nostr.Nip19.decode(payload: keys.nsec).data,
    keys.public,
  );

  List<List<dynamic>> reqsFor(String dTag, String prefix) => [
    for (final req in reqs)
      if ((req[1] as String).startsWith(prefix) && _dTagOf(req) == dTag) req,
  ];

  void emit(NostrEvent event) => session.debugHandleMessage([
    'EVENT',
    _liveSubIds[event.getTagValue('d')],
    event.toJson(),
  ]);

  void closeLive(String dTag, String message) =>
      session.debugHandleMessage(['CLOSED', _liveSubIds[dTag], message]);

  void _onSend(List<dynamic> message) {
    if (message.first == 'EVENT') {
      final event = NostrEvent.fromJson(message[1] as Map<String, dynamic>);
      published.add(event);
      stored.add(event);
      final ok = ['OK', event.id, true, ''];
      final hold = holdOk;
      hold == null
          ? scheduleMicrotask(() => _reply(ok))
          : hold.future.then((_) => _reply(ok));
    }
    if (message.first != 'REQ') return;
    reqs.add(message);
    final subId = message[1] as String;
    if (subId.startsWith('h-')) {
      unawaited(_answerHistory(subId, _dTagOf(message)));
      return;
    }
    _liveSubIds[_dTagOf(message)] = subId;
    final rejection = rejectLive;
    if (rejection == null && withholdEose) return;
    scheduleMicrotask(
      () => _reply(
        rejection == null ? ['EOSE', subId] : ['CLOSED', subId, rejection],
      ),
    );
  }

  Future<void> _answerHistory(String subId, String dTag) async {
    await holdHistory?.future;
    if (historyFailures > 0) {
      historyFailures--;
      return _reply(['CLOSED', subId, 'error: overloaded']);
    }
    final heads = stored.where((e) => e.getTagValue('d') == dTag).toList()
      ..sort(
        (a, b) => a.createdAt != b.createdAt
            ? b.createdAt.compareTo(a.createdAt)
            : a.id.compareTo(b.id),
      );
    if (heads.isNotEmpty) _reply(['EVENT', subId, heads.first.toJson()]);
    _reply(['EOSE', subId]);
  }

  void _reply(List<dynamic> message) => session.debugHandleMessage(message);

  static String _dTagOf(List<dynamic> req) =>
      ((req[2] as Map<String, dynamic>)['#d'] as List).single as String;
}

class ConnectedRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

class _ScriptedSocket extends RelaySocket {
  _ScriptedSocket(this._send)
    : super(
        wsUrl: 'wss://relay.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );

  final void Function(List<dynamic>) _send;

  @override
  void send(List<dynamic> payload) => _send(payload);

  @override
  void dispose() {}
}

/// Preferences whose next [failures] writes return false or throw [error].
class FlakyPrefs extends Fake implements SharedPreferences {
  FlakyPrefs(this._inner);

  final SharedPreferences _inner;
  int failures = 0;
  Object? error;

  @override
  String? getString(String key) => _inner.getString(key);

  @override
  Future<bool> setString(String key, String value) async {
    if (failures == 0) return _inner.setString(key, value);
    failures--;
    if (error == null) return false;
    throw error!;
  }
}

Future<SharedPreferences> freshPrefs() async {
  SharedPreferences.setMockInitialValues({});
  return SharedPreferences.getInstance();
}

int nowSeconds() => DateTime.now().millisecondsSinceEpoch ~/ 1000;

void fakeAsyncTest(String name, void Function(FakeAsync clock) body) =>
    test(name, () => fakeAsync(body));
