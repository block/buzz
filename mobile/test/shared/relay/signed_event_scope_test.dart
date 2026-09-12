import 'dart:async';

import 'package:buzz/shared/auth/event_signer.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);
}

class _Config extends RelayConfigNotifier {
  final String nsec;
  _Config(this.nsec);
  @override
  RelayConfig build() => RelayConfig(baseUrl: 'https://a.example', nsec: nsec);
  void switchCommunity() =>
      state = RelayConfig(baseUrl: 'https://b.example', nsec: nsec);
}

class _Signer implements EventSigner {
  final LocalEventSigner local;
  final ready = Completer<void>();
  _Signer(this.local);
  @override
  String get publicKey => local.publicKey;
  @override
  Future<nostr.Event> sign(UnsignedEvent event) async {
    await ready.future;
    return local.sign(event);
  }
}

class _Socket extends RelaySocket {
  final RelaySessionNotifier session;
  final messages = <List<dynamic>>[];
  void Function()? afterAck;
  _Socket(this.session)
    : super(
        wsUrl: 'wss://recording.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );
  @override
  void send(List<dynamic> payload) {
    messages.add(payload);
    if (payload.first == 'EVENT') {
      session.debugHandleMessage(['OK', (payload[1] as Map)['id'], true, 'ok']);
      afterAck?.call();
    }
  }

  @override
  void dispose() {}
}

class _StatusCache extends UserStatusCacheNotifier {
  final updates = <UserStatus?>[];
  @override
  Map<String, UserStatus?> build() => {};
  @override
  void updateStatus(String pubkey, UserStatus? status) => updates.add(status);
}

class _Harness {
  final keys = nostr.Keys.generate();
  late final config = _Config(keys.nsec);
  final gate = RelayRateLimitGate();
  final cache = _StatusCache();
  late final session = RelaySessionNotifier(rateLimitGate: gate);
  late final container = ProviderContainer(
    overrides: [
      userStatusCacheProvider.overrideWith(() => cache),
      authProvider.overrideWith(_Auth.new),
      relayConfigProvider.overrideWith(() => config),
      relaySessionProvider.overrideWith(() => session),
    ],
  );
  late final a = _Socket(session);

  Future<void> initialize() async {
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    container.read(relaySessionProvider);
    session.debugAttachSocketForTest(a);
  }

  _Socket switchCommunity() {
    config.switchCommunity();
    container.read(relaySessionProvider);
    expect(container.read(relaySessionProvider.notifier), same(session));
    final b = _Socket(session);
    session.debugAttachSocketForTest(b);
    return b;
  }

  SignedEventRelay relay(EventSigner signer) =>
      SignedEventRelay.withSigner(session: session, signer: signer);
}

Future<NostrEvent> _submit(
  SignedEventRelay relay, {
  void Function(NostrEvent)? onSigned,
}) => relay.submit(
  kind: 40002,
  content: 'belongs to A',
  tags: const [
    ['h', 'channel-a'],
  ],
  onSigned: onSigned,
);

void main() {
  test(
    'deferred A signing cannot callback or publish on same-key community B',
    () async {
      final h = _Harness();
      await h.initialize();
      final signer = _Signer(LocalEventSigner(h.keys.secret));
      var callbacks = 0;
      final pending = _submit(h.relay(signer), onSigned: (_) => callbacks++);
      final assertion = expectLater(
        pending,
        throwsA(isA<RelaySessionSupersededError>()),
      );
      final b = h.switchCommunity();
      signer.ready.complete();
      await assertion;
      expect(h.a.messages, isEmpty);
      expect(
        b.messages,
        isEmpty,
        reason: 'A event must never reach B transport',
      );
      expect(callbacks, 0);
    },
  );

  test(
    'dependency invalidation cancels even before the next session read',
    () async {
      final h = _Harness();
      await h.initialize();
      final signer = _Signer(LocalEventSigner(h.keys.secret));
      var callbacks = 0;
      final pending = _submit(h.relay(signer), onSigned: (_) => callbacks++);
      final assertion = expectLater(
        pending,
        throwsA(isA<RelaySessionSupersededError>()),
      );
      h.config.switchCommunity();
      signer.ready.complete();
      await assertion;
      expect(h.a.messages, isEmpty);
      expect(callbacks, 0);
    },
  );

  test(
    'normal delayed signing callbacks once then publishes exact event',
    () async {
      final h = _Harness();
      await h.initialize();
      final signer = _Signer(LocalEventSigner(h.keys.secret));
      final callbacks = <NostrEvent>[];
      final pending = _submit(
        h.relay(signer),
        onSigned: (event) {
          expect(h.a.messages, isEmpty);
          callbacks.add(event);
        },
      );
      expect(callbacks, isEmpty);
      signer.ready.complete();
      final result = await pending;
      expect(callbacks, hasLength(1));
      expect(h.a.messages.single, ['EVENT', callbacks.single.toJson()]);
      expect(result.id, callbacks.single.id);
      expect(result.content, 'ok');
    },
  );

  test('retained signed relay cannot start new work after rebuild', () async {
    final h = _Harness();
    await h.initialize();
    final relay = h.relay(LocalEventSigner(h.keys.secret));
    final b = h.switchCommunity();
    await expectLater(
      _submit(relay),
      throwsA(isA<RelaySessionSupersededError>()),
    );
    expect(b.messages, isEmpty);
  });

  test(
    'callback switching scope is fenced again at production publish',
    () async {
      final h = _Harness();
      await h.initialize();
      late _Socket b;
      await expectLater(
        _submit(
          h.relay(LocalEventSigner(h.keys.secret)),
          onSigned: (_) => b = h.switchCommunity(),
        ),
        throwsA(isA<RelaySessionSupersededError>()),
      );
      expect(h.a.messages, isEmpty);
      expect(b.messages, isEmpty);
    },
  );

  test('direct publication cannot reuse a retired or foreign lease', () async {
    final h = _Harness();
    await h.initialize();
    final lease = h.session.captureLease();
    final signed = await LocalEventSigner(h.keys.secret).sign(
      UnsignedEvent(
        publicKey: h.keys.public,
        createdAt: 1,
        kind: 1,
        content: '',
        tags: const [],
      ),
    );
    final event = NostrEvent.fromJson(signed.toMap());
    final b = h.switchCommunity();
    for (final invalid in [lease, RelaySessionNotifier().captureLease()]) {
      await expectLater(
        h.session.publish(event, lease: invalid),
        throwsA(isA<RelaySessionSupersededError>()),
      );
      expect(
        () => h.session.sendRaw(['EVENT', event.toJson()], lease: invalid),
        throwsA(isA<RelaySessionSupersededError>()),
      );
    }
    expect(b.messages, isEmpty);
  });

  test(
    'scope retires while production publish waits on rate-limit gate',
    () async {
      final h = _Harness();
      await h.initialize();
      final callback = Completer<void>();
      h.gate.activate(30);
      final pending = _submit(
        h.relay(LocalEventSigner(h.keys.secret)),
        onSigned: (_) => callback.complete(),
      );
      final assertion = expectLater(
        pending,
        throwsA(isA<RelaySessionSupersededError>()),
      );
      await callback.future;
      expect(h.a.messages, isEmpty);
      final b = h.switchCommunity();
      await assertion;
      expect(b.messages, isEmpty);
    },
  );

  test(
    'ordinary connection supersession does not retire the scope lease',
    () async {
      final h = _Harness();
      await h.initialize();
      final signer = _Signer(LocalEventSigner(h.keys.secret));
      final pending = _submit(h.relay(signer));
      h.session.debugSupersedeConnection();
      signer.ready.complete();
      await pending;
      expect(h.a.messages, hasLength(1));
    },
  );

  test(
    'user status direct signer cannot publish across community switch',
    () async {
      final h = _Harness();
      await h.initialize();
      await h.container.read(userStatusProvider.future);
      final pending = h.container
          .read(userStatusProvider.notifier)
          .setStatus('A status', '');
      final assertion = expectLater(
        pending,
        throwsA(isA<RelaySessionSupersededError>()),
      );
      final b = h.switchCommunity();
      await assertion;
      expect(h.a.messages, isEmpty);
      expect(b.messages, isEmpty);
      expect(h.cache.updates, isEmpty);
    },
  );

  test(
    'user status cannot update B state/cache after an A acknowledgement',
    () async {
      final h = _Harness();
      await h.initialize();
      await h.container.read(userStatusProvider.future);
      late _Socket b;
      h.a.afterAck = () => b = h.switchCommunity();
      await expectLater(
        h.container.read(userStatusProvider.notifier).setStatus('A status', ''),
        throwsA(isA<RelaySessionSupersededError>()),
      );
      expect(h.a.messages, hasLength(1));
      expect(b.messages, isEmpty);
      expect(h.cache.updates, isEmpty);
      expect(await h.container.read(userStatusProvider.future), isNull);
    },
  );

  test('disposal while signing cancels without callback or send', () async {
    final h = _Harness();
    await h.initialize();
    final signer = _Signer(LocalEventSigner(h.keys.secret));
    var callbacks = 0;
    final pending = _submit(h.relay(signer), onSigned: (_) => callbacks++);
    final assertion = expectLater(
      pending,
      throwsA(isA<RelaySessionSupersededError>()),
    );
    h.session.debugDispose();
    signer.ready.complete();
    await assertion;
    expect(h.a.messages, isEmpty);
    expect(callbacks, 0);
  });
}
