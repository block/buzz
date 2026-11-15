import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/activity/compose_drafts_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/quick_send_host.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _EmptyChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;

  void background() => state = AppLifecycleState.paused;
}

class _Config extends RelayConfigNotifier {
  _Config(this.config);
  final RelayConfig config;
  @override
  RelayConfig build() => config;
}

/// A relay that is not connected until the test says so, accepts every
/// publish and serves back the DM its open command finds or creates.
class _Relay extends RelaySessionNotifier {
  _Relay(this.owner, this.profile);
  final nostr.Keys owner;
  final NostrEvent profile;
  final published = <NostrEvent>[];

  /// Rejects the next message the way an over-quota relay does.
  var rateLimitNextMessage = false;

  /// Loses the next message's answer, so it may or may not have landed.
  var timeOutNextMessage = false;

  /// Drops the socket just before the next message goes out.
  var dropBeforeNextMessage = false;
  final _stored = <NostrEvent>[];
  static const dmId = 'bestie-dm';

  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  void connect() => state = const SessionState(status: SessionStatus.connected);

  /// Background pauses started; the real session closes the socket after one.
  var pauses = 0;

  @override
  void onAppPaused() => pauses++;

  @override
  void onAppResumed() => connect();

  NostrEvent _sign(int kind, List<List<String>> tags) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: kind,
      content: '',
      secretKey: owner.secret,
      tags: tags,
    ).toMap(),
  );

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [profile];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(0)) return [profile];
    final ds = filter.tags['#d'];
    return _stored.reversed
        .where(
          (e) =>
              filter.kinds.contains(e.kind) &&
              (ds == null || ds.contains(e.getTagValue('d'))),
        )
        .take(filter.limit)
        .toList();
  }

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
  }) async => () {};

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (event.kind == 9 && rateLimitNextMessage) {
      rateLimitNextMessage = false;
      throw Exception('rate-limited: quota exceeded; retry in 0s');
    }
    if (event.kind == 9 && dropBeforeNextMessage) {
      dropBeforeNextMessage = false;
      throw StateError('Relay session is not connected');
    }
    if (event.kind == 9 && timeOutNextMessage) {
      timeOutNextMessage = false;
      throw TimeoutException('no OK');
    }
    published.add(event);
    if (event.kind != 41010) return event;
    // Opening is find-or-create: the relay answers with the one DM.
    if (!_stored.any((e) => e.getTagValue('d') == dmId)) {
      final peer = event.getTagValue('p')!;
      _stored.add(
        _sign(39000, [
          ['d', dmId],
          ['name', 'DM'],
          ['t', 'dm'],
          ['private'],
          ['hidden'],
          ['p', owner.public],
          ['p', peer],
        ]),
      );
      _stored.add(
        _sign(39002, [
          ['d', dmId],
          ['p', owner.public, '', 'member'],
          ['p', peer, '', 'bot'],
        ]),
      );
    }
    return NostrEvent(
      id: event.id,
      pubkey: event.pubkey,
      createdAt: event.createdAt,
      kind: event.kind,
      tags: event.tags,
      content: 'response:${jsonEncode({'channel_id': dmId, 'created': true})}',
      sig: event.sig,
    );
  }
}

void main() {
  testWidgets('Siri quick send posts in the Bestie DM', (tester) async {
    const channel = MethodChannel('buzz/quick_send');
    final platformCalls = <String>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      platformCalls.add(call.method);
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final owner = nostr.Keys.generate(), bestie = nostr.Keys.generate();
    final digest = SHA256Digest()
        .process(
          Uint8List.fromList(utf8.encode('nostr:agent-auth:${bestie.public}:')),
        )
        .map((v) => v.toRadixString(16).padLeft(2, '0'))
        .join();
    final profile = NostrEvent.fromJson(
      nostr.Event.from(
        kind: 0,
        content: jsonEncode({'display_name': 'Bestie'}),
        secretKey: bestie.secret,
        tags: [
          [
            'auth',
            owner.public,
            '',
            nostr.Schnorr.sign(secretKey: owner.secret, message: digest),
          ],
        ],
      ).toMap(),
    );
    final relay = _Relay(owner, profile);
    String? chosenBestie;
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          relayConfigProvider.overrideWith(
            () => _Config(
              RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
            ),
          ),
          relaySessionProvider.overrideWith(() => relay),
          defaultAgentProvider.overrideWith(
            (ref) => Stream.value(chosenBestie),
          ),
          channelsProvider.overrideWith(_EmptyChannels.new),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const QuickSendHost(),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(platformCalls, ['ready'], reason: 'Siri can send once Home is up');

    // Siri calls the host the way QuickSend.swift does and gets its reply.
    Future<(Object?, PlatformException?)> siri(
      String method, [
      Object? arguments,
    ]) async {
      Object? result;
      PlatformException? failure;
      var replied = false;
      tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        channel.name,
        channel.codec.encodeMethodCall(MethodCall(method, arguments)),
        (reply) {
          try {
            result = channel.codec.decodeEnvelope(reply!);
          } on PlatformException catch (e) {
            failure = e;
          }
          replied = true;
        },
      );
      // Ownership verification runs in a real isolate.
      for (var i = 0; i < 100 && !replied; i++) {
        await tester.pump(const Duration(milliseconds: 50));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
      }
      expect(replied, isTrue, reason: '$method never answered');
      return (result, failure);
    }

    final container = ProviderScope.containerOf(
      tester.element(find.byType(QuickSendHost)),
    );
    container.read(relaySessionProvider);
    relay.connect();
    await tester.pump();

    final (_, noBestie) = await siri('bestie');
    expect(noBestie?.code, 'failed');
    expect(noBestie?.message, 'Choose a Bestie in Buzz first.');

    chosenBestie = bestie.public;
    container.invalidate(defaultAgentProvider);
    await tester.pump();
    final (named, _) = await siri('bestie');
    final scope = (named! as Map)['scope'];
    expect(named, {'pubkey': bestie.public, 'name': 'Bestie', 'scope': scope});

    // A confirmation from before a community switch can't send here.
    final (_, switched) = await siri('send', {
      'text': 'Remind me to call Sam',
      'bestie': bestie.public,
      'scope': 'https://other.test $scope',
    });
    expect(switched?.message, contains('switched communities'));

    final (_, blank) = await siri('send', {
      'text': '  \n ',
      'bestie': bestie.public,
      'scope': scope,
    });
    expect(blank?.code, 'failed');
    expect(blank?.message, 'Say a message.');
    expect(relay.published, isEmpty);

    // Buzz sat in the background long enough for its socket to close.
    (container.read(appLifecycleProvider.notifier) as _Lifecycle).background();
    relay.state = const SessionState(status: SessionStatus.disconnected);
    final pausesBefore = relay.pauses;
    // Its catch-up used the write quota, so the first try is turned away.
    relay.rateLimitNextMessage = true;
    await tester.pumpAndSettle();
    final (_, failure) = await siri('send', {
      'text': '  Remind me to call Sam  ',
      'bestie': bestie.public,
      'scope': scope,
    });
    expect(failure, isNull);
    expect(
      relay.pauses,
      pausesBefore + 1,
      reason: 'The send hands the session back to its background pause',
    );

    final open = relay.published.singleWhere((e) => e.kind == 41010);
    expect(open.pubkey, owner.public);
    expect(open.tags, [
      ['p', bestie.public],
    ]);
    final first = relay.published.singleWhere((e) => e.kind == 9);
    expect(first.content, 'Remind me to call Sam');
    expect(first.pubkey, owner.public);
    expect(first.getTagValue('h'), _Relay.dmId);
    expect(first.tags, contains(equals(['p', bestie.public])));

    // A sent message leaves the DM's draft as it was.
    final drafts = container.read(composeDraftsProvider.notifier);
    drafts.save(
      key: _Relay.dmId,
      channelId: _Relay.dmId,
      text: 'Half a @Sam',
      mentionKeys: {'Sam': 'sam-pubkey'},
    );

    // A second quick send goes to the same DM.
    final (_, again) = await siri('send', {
      'text': 'And buy milk',
      'bestie': bestie.public,
      'scope': scope,
    });
    expect(again, isNull);
    final messages = relay.published.where((e) => e.kind == 9).toList();
    expect(messages.map((e) => e.content), [
      'Remind me to call Sam',
      'And buy milk',
    ]);
    expect(messages.map((e) => e.getTagValue('h')).toSet(), {_Relay.dmId});
    expect(drafts.textFor(_Relay.dmId), 'Half a @Sam');

    // When Buzz can't tell whether a send landed, the text stays in the DM's
    // draft, after the person's own, and Siri says to check before resending.
    relay.timeOutNextMessage = true;
    final (_, unsure) = await siri('send', {
      'text': 'Call back',
      'bestie': bestie.public,
      'scope': scope,
    });
    expect(
      unsure?.message,
      "Buzz isn't sure that sent. Check your Bestie DM before trying again.",
    );
    final kept = drafts.draftFor(_Relay.dmId)!;
    expect(kept.text, 'Half a @Sam\n\nCall back');
    expect(kept.mentionKeys, {'Sam': 'sam-pubkey'});

    // A socket that drops just before the message goes out keeps it too.
    relay.dropBeforeNextMessage = true;
    final (_, dropped) = await siri('send', {
      'text': 'Then Sam',
      'bestie': bestie.public,
      'scope': scope,
    });
    expect(dropped?.message, contains('Check your Bestie DM'));
    expect(drafts.textFor(_Relay.dmId), 'Half a @Sam\n\nCall back\n\nThen Sam');

    // Leaving Home, for example on sign-out, takes the handler with it.
    await tester.pumpWidget(const SizedBox());
    expect(platformCalls, ['ready', 'unready']);
    // Let the sent messages' local animations finish.
    await tester.pump(const Duration(seconds: 10));
  });
}
