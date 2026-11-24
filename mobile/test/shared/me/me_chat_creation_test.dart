import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/me/me_chat_title.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  // Desktop renames a chat only while its name is still the title desktop
  // derives from the first message, so these are desktop's cases for
  // `startingChatTitle`, from `src/features/solo/chat-title.test.ts` in
  // block/buzz-app at 0785a47f26, written as mobile publishes a message:
  // its content and `imeta` tags. Change the two together.
  //
  // Not ported: cases about desktop's composer state (recipient chips and
  // staged GIFs), and `isStartingChatTitle`, which only desktop asks.
  test('first-message title follows desktop rules', () {
    final media = 'https://relay.example/media/${'a' * 64}';
    final source = 'buzz://message?channel=design&id=${'c' * 64}';
    List<String> imeta(String url, String type, [String? filename]) => [
      'imeta',
      'url $url',
      'm $type',
      if (filename != null) 'filename $filename',
    ];
    final video = imeta('$media.mp4', 'video/mp4');
    final image = imeta('$media.png', 'image/png');
    final photo = imeta('$media.jpg', 'image/jpeg');
    final file = imeta('$media.txt', 'text/plain', 'notes.txt');
    for (final (message, tags, title) in <(String, List<List<String>>, String)>[
      // uses readable channel labels before truncating the title
      (
        r'what is this about [\#design-and-research-work](buzz://channel/b17e217e-1234-4567-8901-123456789abc)',
        [],
        'what is this about #design-and-research-work',
      ),
      (
        'Read [guide](https://example.com/${'x' * 120}) then discuss',
        [],
        'Read guide then discuss',
      ),
      // reads escaped and formatted labels across multiple links
      (
        r'Compare [**A** \[one\]](https://example.com/a) with [`B` &amp; C](https://example.com/b)',
        [],
        'Compare A [one] with B & C',
      ),
      (
        'Read [the guide][ref]\n\n[ref]: https://example.com',
        [],
        'Read the guide [ref]: https://example.com',
      ),
      // preserves code, malformed links and unrelated Markdown source
      for (final text in [
        'Keep `[label](https://example.com)` literal',
        'Keep [label](unfinished',
        r'Keep **bold** and literal \# text',
      ])
        (text, [], text),
      // truncates readable labels at 80 Unicode code points
      ('[${'🐝' * 81}](https://example.com)', [], '🐝' * 80),
      // titles a linked chat from the prompt after its leading source link
      ('$source Investigate this failure', [], 'Investigate this failure'),
      ('$source\n$source Compare both', [], 'Compare both'),
      // never titles a link-only prompt with the link, but keeps later links
      ('buzz://channel/design', [], 'Chat'),
      ('Look at buzz://channel/design', [], 'Look at buzz://channel/design'),
      // titles a message that is only media by what it sent
      ('\n![video]($media.mp4)', [video], 'Video'),
      ('\n![image]($media.png)', [image], 'Image'),
      ('\n[notes.txt]($media.txt)', [file], 'Attachment'),
      (
        '\n![image]($media.png)\n![image]($media.jpg)',
        [image, photo],
        '2 images',
      ),
      (
        '\n![image]($media.png)\n![video]($media.mp4)\n[notes.txt]($media.txt)',
        [image, video, file],
        '3 attachments',
      ),
      ('![wave](https://gif.example/1)', [], 'Image'),
      ('![video]($media.mp4)', [], 'Video'),
      ('![a]($media.png)\n![b]($media.jpg)', [], '2 images'),
      ('[notes.txt](<$media.txt>)', [file], 'Attachment'),
      // titles text sent with media by the text alone
      ('See attached\n![video]($media.mp4)', [video], 'See attached'),
      (
        'What is this?\n\n![shot.png](<$media.png>)\n\n[notes.txt](<$media.txt>)',
        [image, file],
        'What is this?',
      ),
      // titles a message that is only web links by their site, never the
      // address
      ('https://www.example.com/a/b?c=d#e', [], 'Link to example.com'),
      ('<https://example.com/a>', [], 'Link to example.com'),
      (
        '[https://example.com/a](https://example.com/a)',
        [],
        'Link to example.com',
      ),
      ('https://a.example/1\nhttps://b.example/2', [], '2 links'),
      ('[Release notes](https://example.com/a)', [], 'Release notes'),
      (
        'Summarize https://example.com/a',
        [],
        'Summarize https://example.com/a',
      ),
      ('https://example.com/${'x' * 120}', [], 'Link to example.com'),

      // Mobile's own cases. Plain text, and what its composer attaches.
      ('  Plan\n  my trip  ', [], 'Plan my trip'),
      ('@Alice help me', [], '@Alice help me'),
      ('### Heading', [], 'Heading'),
      ('🐝' * 90, [], '🐝' * 80),
      (
        '\n![image]($media.png)\n![image]($media.jpg)\n![image]($media.webp)',
        [image, photo, imeta('$media.webp', 'image/webp')],
        '3 images',
      ),
      (
        '\n![video]($media.mp4)\n![video]($media.mov)',
        [video, imeta('$media.mov', 'video/quicktime')],
        '2 videos',
      ),
      // Desktop words audio, and a voice note either build records, as any
      // other file.
      (
        '\n![audio]($media.m4a)',
        [imeta('$media.m4a', 'audio/mp4')],
        'Attachment',
      ),
      (
        '\n[voice-note-1732.mp4]($media.mp4)',
        [imeta('$media.mp4', 'video/mp4', 'voice-note-1732.mp4')],
        'Attachment',
      ),
      // The same file attached twice is one attachment.
      ('\n![image]($media.png)\n![image]($media.png)', [image, image], 'Image'),
      // A link to a file that was not attached keeps its label.
      ('[notes.txt]($media.txt)', [], 'notes.txt'),
      ('Look\n![video]($media.mp4)\nhere', [video], 'Look here'),
      (
        '```\nlist[0](x) ![a](https://x.example/a.png)\n```',
        [],
        '``` list[0](x) ![a](https://x.example/a.png) ```',
      ),
      (
        r'Not \[a link](https://example.com)',
        [],
        r'Not \[a link](https://example.com)',
      ),
      // Media counts only at an HTTPS address, as desktop reads it.
      (
        '\n![video](http://relay.example/a.mp4)',
        [imeta('http://relay.example/a.mp4', 'video/mp4')],
        'Chat',
      ),
      // Start Solo chat's quoted source is context: mobile's rule alone.
      ('> Ship it Friday\n>\n> ok?\n\nSummarize this', [], 'Summarize this'),
      ('> Ship it Friday\n> ok?\n\n', [], 'Ship it Friday ok?'),
    ]) {
      expect(startingMeChatTitle(message, tags: tags), title, reason: message);
    }
  });
  test(
    'accepted first message survives a lost acknowledgement and restart',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final owner = nostr.Keys.generate();
      final agent = nostr.Keys.generate().public;
      final relay = _MessageRelay();
      final config = RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: owner.nsec,
      );
      MeSync service() => MeSync(relay, config, prefs: prefs);
      final message = NostrEvent.fromJson(
        signNostrEvent(
          kind: 9,
          content: 'Plan my trip',
          tags: [
            ['h', 'chat'],
            ['p', agent],
          ],
          secretKey: owner.secret,
        ).toMap(),
      );
      final intent = MeChatCreation(
        'chat',
        'Plan my trip',
        agent,
        initialMessage: message,
      );
      Future<String> send(MeChatCreation intent) => intent.complete(
        service(),
        findChannel: () async => 'chat',
        createChannel: () async => throw StateError('already created'),
        hasAgent: () async => true,
        addAgent: () async => throw StateError('already joined'),
        isPlaced: () async => true,
        place: () async => throw StateError('already placed'),
      );
      await expectLater(send(intent), throwsStateError);
      await prefs.reload();
      final pending = MeChatCreation.pending(service())!;
      expect(pending.initialMessage?.toJson(), message.toJson());
      expect(
        prefs.getString(MeChatCreation.storageKey(service())),
        isNot(contains('Plan my trip')),
      );
      await expectLater(
        MeChatCreation('chat', intent.name, agent).save(service()),
        throwsStateError,
      );
      expect(await send(pending), 'chat');
      expect(relay.attempts, [message.id, message.id]);
      expect(relay.messages.length, 1);
      expect(MeChatCreation.pending(service()), isNull);
    },
  );
  for (final accepted in [false, true]) {
    test(
      'expired first message resumes without duplication: accepted=$accepted',
      () async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final owner = nostr.Keys.generate();
        final relay = _MessageRelay();
        final sync = MeSync(
          relay,
          RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
          prefs: prefs,
        );
        final old = NostrEvent.fromJson(
          signNostrEvent(
            kind: 9,
            content: 'Yesterday’s draft',
            tags: [
              ['h', 'chat'],
              ['p', 'agent'],
            ],
            secretKey: owner.secret,
            createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 86400,
          ).toMap(),
        );
        if (accepted) relay.messages[old.id] = old;
        final intent = MeChatCreation(
          'chat',
          'Yesterday’s draft',
          'agent',
          initialMessage: old,
        );
        Future<String> run(MeChatCreation intent) => intent.complete(
          sync,
          findChannel: () async => 'chat',
          createChannel: () async => throw StateError('already created'),
          hasAgent: () async => true,
          addAgent: () async => throw StateError('already joined'),
          isPlaced: () async => true,
          place: () async => throw StateError('already placed'),
        );
        if (!accepted) {
          await expectLater(run(intent), throwsStateError);
          await prefs.reload();
          final pending = MeChatCreation.pending(sync)!;
          expect(pending.initialMessage!.id, isNot(old.id));
          expect(pending.initialMessage!.content, old.content);
          expect(pending.initialMessage!.pubkey, owner.public);
          expect(pending.initialMessage!.tags, old.tags);
          expect(await run(pending), 'chat');
          expect(relay.attempts.toSet(), {pending.initialMessage!.id});
        } else {
          expect(await run(intent), 'chat');
          expect(relay.attempts, isEmpty);
        }
        expect(relay.messages.length, 1);
        expect(MeChatCreation.pending(sync), isNull);
      },
    );
  }
  test(
    'retry confirms persistence after a failed cached journal write',
    () async {
      SharedPreferences.setMockInitialValues({});
      final store = _FailingStore();
      final previousStore = SharedPreferencesStorePlatform.instance;
      SharedPreferencesStorePlatform.instance = store;
      addTearDown(
        () => SharedPreferencesStorePlatform.instance = previousStore,
      );
      final prefs = await SharedPreferences.getInstance();
      final sync = MeSync(
        RelaySessionNotifier(),
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Keys.generate().nsec,
        ),
        prefs: prefs,
      );
      const intent = MeChatCreation('id', 'Name', 'agent');
      var reachedRelay = false;
      Future<String> run() => intent.complete<String>(
        sync,
        findChannel: () async {
          reachedRelay = true;
          await prefs.reload();
          expect(MeChatCreation.pending(sync)?.id, intent.id);
          return null;
        },
        createChannel: () async => throw StateError('Simulated process stop'),
        hasAgent: () async => false,
        addAgent: () async {},
        isPlaced: () async => false,
        place: () async {},
      );
      await expectLater(run(), throwsStateError);
      expect(reachedRelay, isFalse);
      expect(
        MeChatCreation.pending(sync)?.id,
        intent.id,
      ); // Failed write remains cached.
      await expectLater(run(), throwsStateError);
      expect(reachedRelay, isTrue);
      await prefs.reload();
      expect(MeChatCreation.pending(sync)?.id, intent.id);
    },
  );
  test('missing durable storage blocks every relay side effect', () async {
    final sync = MeSync(
      RelaySessionNotifier(),
      RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: '0' * 63 + '1',
        ),
      ),
    );
    var touchedRelay = false;
    await expectLater(
      const MeChatCreation('id', 'Name', 'agent').complete<String>(
        sync,
        findChannel: () async {
          touchedRelay = true;
          return null;
        },
        createChannel: () async => 'id',
        hasAgent: () async => false,
        addAgent: () async {},
        isPlaced: () async => false,
        place: () async {},
      ),
      throwsStateError,
    );
    expect(touchedRelay, isFalse);
  });
  test(
    'discard preserves partial relay state and permits a different intent offline',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final config = RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: '0' * 63 + '1',
        ),
      );
      final online = MeSync(RelaySessionNotifier(), config, prefs: prefs);
      const intent = MeChatCreation('old', 'Old', 'revoked-agent');
      await intent.save(online);
      final offline = MeSync(
        RelaySessionNotifier(),
        config,
        prefs: prefs,
        connected: false,
      );
      await expectLater(
        const MeChatCreation('wrong', 'Old', 'revoked-agent').discard(offline),
        throwsStateError,
      );
      expect(MeChatCreation.pending(offline)?.id, 'old');
      await intent.discard(offline);
      expect(MeChatCreation.pending(offline), isNull);
      await const MeChatCreation('new', 'New', 'other-agent').save(online);
      expect(MeChatCreation.pending(online)?.id, 'new');
    },
  );
  for (final crashAfter in [0, 1, 2, 3]) {
    test('restart resumes the same chat after boundary $crashAfter', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final config = RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: '0' * 63 + '1',
        ),
      );
      MeSync service() => MeSync(RelaySessionNotifier(), config, prefs: prefs);
      const intent = MeChatCreation(
        'same-id',
        'Private name',
        'agent',
        project: 'mobile',
      );
      var created = false, member = false, placed = false, crash = true;
      final writes = [0, 0, 0];
      void boundary(int point) {
        expect(MeChatCreation.pending(service())?.id, intent.id);
        if (crash && crashAfter == point) throw StateError('process stopped');
      }

      Future<String> run(MeSync sync, MeChatCreation intent) =>
          intent.complete<String>(
            sync,
            findChannel: () async {
              boundary(0);
              return created ? intent.id : null;
            },
            createChannel: () async {
              created = true;
              writes[0]++;
              boundary(1);
              return intent.id;
            },
            hasAgent: () async => member,
            addAgent: () async {
              member = true;
              writes[1]++;
              boundary(2);
            },
            isPlaced: () async => placed,
            place: () async {
              placed = true;
              writes[2]++;
              boundary(3);
            },
          );
      await expectLater(run(service(), intent), throwsStateError);
      final restarted = service();
      final pending = MeChatCreation.pending(restarted)!;
      expect(pending.name, intent.name);
      expect(pending.agent, intent.agent);
      expect(pending.project, intent.project);
      await expectLater(
        MeChatCreation(
          intent.id,
          intent.name,
          intent.agent,
          project: 'other',
        ).save(restarted),
        throwsStateError,
      );
      expect(
        prefs.getString(MeChatCreation.storageKey(restarted)),
        isNot(contains(intent.name)),
      );
      crash = false;
      expect(await run(restarted, pending), intent.id);
      expect(writes, [1, 1, 1]);
      expect(MeChatCreation.pending(restarted), isNull);
      final other = MeSync(
        RelaySessionNotifier(),
        RelayConfig(baseUrl: 'https://other.test', nsec: config.nsec),
        prefs: prefs,
      );
      expect(MeChatCreation.pending(other), isNull);
    });
  }
}

class _FailingStore extends InMemorySharedPreferencesStore {
  _FailingStore() : super.empty();
  bool failNext = true;
  @override
  Future<bool> setValue(String valueType, String key, Object value) async {
    if (failNext && key.startsWith('flutter.me-chat-creation:')) {
      failNext = false;
      return false;
    }
    return super.setValue(valueType, key, value);
  }
}

class _MessageRelay extends RelaySessionNotifier {
  final messages = <String, NostrEvent>{};
  final attempts = <String>[];
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [
    for (final message in messages.values)
      if (filter.ids!.contains(message.id)) message,
  ];
  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
    Future<bool> Function()? mayPublish,
  }) async {
    attempts.add(event.id);
    messages[event.id] = event;
    if (attempts.length == 1) throw StateError('Acknowledgement lost');
    return event;
  }
}
