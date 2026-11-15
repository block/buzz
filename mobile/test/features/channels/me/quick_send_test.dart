import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_sync.dart';
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
/// publish and serves back the channel metadata and roster its commands
/// create.
class _Relay extends RelaySessionNotifier {
  _Relay(this.owner, this.profile);
  final nostr.Keys owner;
  final NostrEvent profile;
  final published = <NostrEvent>[];

  /// Rejects the next message the way an over-quota relay does.
  var rateLimitNextMessage = false;
  final _stored = <NostrEvent>[];

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
    published.add(event);
    _stored.add(event);
    final id = event.getTagValue('h');
    if (event.kind == 9007) {
      _stored.add(
        _sign(39000, [
          ['d', id!],
          ['name', event.getTagValue('name')!],
          ['t', 'buzz.solo'],
          ['channel_type', 'stream'],
          ['visibility', 'private'],
        ]),
      );
      _stored.add(
        _sign(39002, [
          ['d', id],
          ['p', owner.public, 'owner'],
        ]),
      );
    } else if (event.kind == 9000) {
      _stored.add(
        _sign(39002, [
          ['d', id!],
          ['p', owner.public, 'owner'],
          ['p', event.getTagValue('p')!, 'bot'],
        ]),
      );
    }
    return event;
  }
}

void main() {
  testWidgets('Siri quick send starts a Solo chat with the Bestie', (
    tester,
  ) async {
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

    // The platform calls `send`; it gets the reply Siri will show.
    Future<PlatformException?> siriSends(Object? text) async {
      PlatformException? failure;
      var replied = false;
      tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        channel.name,
        channel.codec.encodeMethodCall(MethodCall('send', text)),
        (reply) {
          try {
            channel.codec.decodeEnvelope(reply!);
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
      expect(replied, isTrue, reason: 'The send never answered');
      return failure;
    }

    final container = ProviderScope.containerOf(
      tester.element(find.byType(QuickSendHost)),
    );
    container.read(relaySessionProvider);
    relay.connect();
    await tester.pump();

    final noBestie = await siriSends('Remind me to call Sam');
    expect(noBestie?.code, 'failed');
    expect(noBestie?.message, 'Choose a Bestie in Buzz first.');
    expect(relay.published, isEmpty);

    final blank = await siriSends('  \n ');
    expect(blank?.code, 'empty');
    expect(relay.published, isEmpty);

    chosenBestie = bestie.public;
    container.invalidate(defaultAgentProvider);
    // Buzz sat in the background long enough for its socket to close.
    (container.read(appLifecycleProvider.notifier) as _Lifecycle).background();
    relay.state = const SessionState(status: SessionStatus.disconnected);
    final pausesBefore = relay.pauses;
    // Its catch-up used the write quota, so the first try is turned away.
    relay.rateLimitNextMessage = true;
    await tester.pumpAndSettle();
    expect(await siriSends('  Remind me to call Sam  '), isNull);
    expect(
      relay.pauses,
      pausesBefore + 1,
      reason: 'The send hands the session back to its background pause',
    );

    final create = relay.published.singleWhere((e) => e.kind == 9007);
    expect(create.pubkey, owner.public);
    expect(create.tags, contains(equals(['t', 'buzz.solo'])));
    expect(create.tags, contains(equals(['visibility', 'private'])));
    final chatId = create.getTagValue('h')!;
    final invite = relay.published.singleWhere((e) => e.kind == 9000);
    expect(invite.getTagValue('h'), chatId);
    expect(invite.getTagValue('p'), bestie.public);
    final first = relay.published.singleWhere((e) => e.kind == 9);
    expect(first.content, 'Remind me to call Sam');
    expect(first.pubkey, owner.public);
    expect(first.getTagValue('h'), chatId);
    expect(first.tags, contains(equals(['p', bestie.public])));
    expect(
      relay.published.last,
      first,
      reason: 'The message goes out after the chat and its Bestie exist',
    );
    expect(
      MeChatCreation.pending(container.read(meSyncProvider)),
      isNull,
      reason: 'The finished chat leaves nothing to resume',
    );

    // Leaving Home, for example on sign-out, takes the handler with it.
    await tester.pumpWidget(const SizedBox());
    expect(platformCalls, ['ready', 'unready']);
  });
}
