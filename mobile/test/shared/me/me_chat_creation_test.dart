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
  // A first message as the composer publishes it: the trimmed text, then a
  // line and an `imeta` tag for each attachment.
  (String, List<List<String>>) published(
    String text, [
    List<BlobDescriptor> attachments = const [],
  ]) => (
    [text, for (final sent in attachments) sent.toMarkdownImage()].join('\n'),
    [for (final sent in attachments) sent.toImetaTag()],
  );
  const media = 'https://relay.example/media';
  BlobDescriptor upload(String type, String extension, [String? filename]) {
    final hash = extension.codeUnitAt(1).toRadixString(16) * 32;
    return BlobDescriptor(
      url: '$media/$hash$extension',
      sha256: hash,
      size: 1024,
      type: type,
      uploaded: 0,
      filename: filename,
    );
  }

  final photo = upload('image/jpeg', '.jpg');
  final image = upload('image/png', '.png');
  final video = upload('video/mp4', '.mp4');
  final pdf = upload('application/pdf', '.pdf', 'report.pdf');

  test('a Solo chat is named from its first message', () {
    final link = 'https://example.com/${'x' * 120}';
    final source = 'buzz://message?channel=design&id=${'c' * 64}';
    for (final ((message, tags), name) in [
      // Text is the name as written, flattened and cut to 80 characters.
      (published('Draft the launch checklist'), 'Draft the launch checklist'),
      (published('  Plan\n  my trip  '), 'Plan my trip'),
      (published('@Alice help me'), '@Alice help me'),
      (published('### Heading'), 'Heading'),
      (published('🐝' * 90), '🐝' * 80),
      // A control character is flattened as whitespace is, so none is stored.
      (published('Fix \x1b[31mred\x1b[0m output'), 'Fix [31mred [0m output'),
      (published('a\x00b'), 'a b'),
      (published('Hello\u0085world'), 'Hello world'),
      (published('\x00https://example.com/a'), 'Link to example.com'),
      // The relay trims U+0085 as whitespace, and refuses the empty name left.
      (published('# \u0085 #'), 'Chat'),
      // Markdown is not read: links, labels, emphasis and code stay.
      for (final text in [
        '[Release notes](https://example.com/notes)',
        '[a](https://example.com/x(1(2))) [b](https://example.com)',
        r'[literal \![logo](https://example.com/logo.png)](https://example.com)',
        '[https://example.com/a](https://example.com/a)',
        '![wave](https://gif.example/1.gif)',
        r'Keep **bold**, `code` and literal \# text',
        'Summarize https://example.com/a',
        'Send this to <dev@example.org>',
      ])
        (published(text), text),
      (
        published('Read [guide]($link) then'),
        'Read [guide]($link) then'.substring(0, 80),
      ),

      // Only attachments: what was sent, by each tag's type.
      (published('', [photo]), 'Image'),
      (published('', [video]), 'Video'),
      (published('', [pdf]), 'Attachment'),
      (published('', [upload('audio/mp4', '.m4a')]), 'Attachment'),
      (
        published('', [upload('video/mp4', '.mp4', 'voice-note-1732.mp4')]),
        'Attachment',
      ),
      (published('', [photo, image]), '2 images'),
      (published('', [video, upload('video/quicktime', '.mov')]), '2 videos'),
      (published('', [photo, video, pdf]), '3 attachments'),
      // A file name is written into its line with its brackets escaped.
      (
        published('', [upload('text/plain', '.txt', 'a [1].txt')]),
        'Attachment',
      ),
      // The same file attached twice has one address, and is one attachment.
      (published('', [photo, photo]), 'Image'),
      // A type that does not settle the kind leaves it to the file name.
      (published('', [upload('application/octet-stream', '.png')]), 'Image'),
      // An address whose path does not decode is still matched and typed.
      for (final filename in [null, 'clip.mp4'])
        (
          (
            '\n![video]($media/%FF%FE.mp4)',
            [
              [
                'imeta',
                'url $media/%FF%FE.mp4',
                'm video/mp4',
                if (filename != null) 'filename $filename',
              ],
            ],
          ),
          'Video',
        ),
      for (final filename in [null, 'clip.mp4'])
        (
          (
            '\n[file]($media/%FF%ZZ.mp4)',
            [
              [
                'imeta',
                'url $media/%FF%ZZ.mp4',
                if (filename != null) 'filename $filename',
              ],
            ],
          ),
          'Video',
        ),
      (
        (
          '\n[file]($media/%FF%ZZ)',
          [
            ['imeta', 'url $media/%FF%ZZ'],
          ],
        ),
        'Attachment',
      ),

      // Text sent with attachments: the text alone.
      (published('Look at this', [photo]), 'Look at this'),
      (published('What is this?', [image, pdf]), 'What is this?'),
      (
        published('Send this to <dev@example.org>', [photo]),
        'Send this to <dev@example.org>',
      ),
      (published('https://example.com/a', [photo]), 'Link to example.com'),
      // Only lines the composer appended are removed: the trailing ones at
      // an attachment's own address.
      (
        published('![image]($media/other.jpg)', [photo]),
        '![image]($media/other.jpg)',
      ),
      (
        ('${photo.toMarkdownImage()}\nLook', [photo.toImetaTag()]),
        '${photo.toMarkdownImage()} Look'.substring(0, 80),
      ),
      (
        (photo.toMarkdownImage(), const <List<String>>[]),
        photo.toMarkdownImage().substring(0, 80),
      ),
      // The composer ends a message with its attachment lines, so a line
      // with a space after it was typed, and is text.
      (
        ('${photo.toMarkdownImage()} ', [photo.toImetaTag()]),
        photo.toMarkdownImage().substring(0, 80),
      ),

      // Only web addresses: the site of one, or how many.
      (
        published('https://www.example.com/docs/getting-started'),
        'Link to example.com',
      ),
      (published('<https://example.com/a>'), 'Link to example.com'),
      (published('HTTP://Example.COM'), 'Link to example.com'),
      (published(link), 'Link to example.com'),
      (published('https://bücher.example/a'), 'Link to bücher.example'),
      (published('https://[2001:db8::1]:8080/a'), 'Link to [2001:db8::1]'),
      (published('https://a.example/1\n<https://b.example/2>'), '2 links'),
      (published('https://${'a' * 90}.example'), 'Link to ${'a' * 72}'),
      // A site that cannot be read is never guessed at, and never throws.
      for (final address in [
        'https://%FF.example/path',
        'https://%00.example',
        'https://%1B%5B31m.example',
        'https://%C2%9F.example',
        'https://www./a',
        'https:///a',
        'https://[',
      ])
        (published(address), 'Link'),
      for (final text in ['<https://example.com/a', 'ftp://example.com/a'])
        (published(text), text),

      // Start Solo chat seeds a source link or a block quote before the
      // prompt. Neither is the name when a prompt follows.
      (published('$source\n\nInvestigate this'), 'Investigate this'),
      (published('$source\n$source Compare both'), 'Compare both'),
      (published('$source\n\n'), 'Chat'),
      (published('$source\n\n', [photo]), 'Image'),
      (published('Look at $source'), 'Look at $source'.substring(0, 80)),
      (
        published('> Ship it Friday\n>\n> ok?\n\nSummarize this'),
        'Summarize this',
      ),
      (published('> Ship it Friday\n> ok?\n\n'), 'Ship it Friday ok?'),
      (published('> Ship it Friday\n> ok?', [photo]), 'Ship it Friday ok?'),

      // Nothing to name.
      for (final text in ['', '  \n ', '###']) (published(text), 'Chat'),
      (published('#', [video]), 'Video'),
    ]) {
      final title = startingMeChatTitle(message, tags: tags);
      expect(title, name, reason: message);
      expect(title, isNot(matches(r'[\x00-\x1f\x7f-\x9f]')), reason: message);
    }
  });

  // The name is derived on the UI isolate in the Send tap, from a message of
  // up to 8 KB. Reading these as Markdown took seconds.
  test('a Solo chat name takes time linear in its first message', () {
    for (final message in [
      '[x](${' ' * 8000}',
      '[x](${' ' * 8000}y',
      r'\[' * 4000,
      '[x](' * 2000,
      for (final mark in ['**', '*', '_'])
        '[${'${mark}a ' * 2000}](https://example.com)',
      'buzz://x ' * 900,
      '>\n' * 4000,
      '\n' * 8000,
    ]) {
      final clock = Stopwatch()..start();
      for (var run = 0; run < 20; run++) {
        startingMeChatTitle(
          '$message\n${photo.toMarkdownImage()}',
          tags: [photo.toImetaTag()],
        );
      }
      expect(
        clock.elapsed,
        lessThan(const Duration(seconds: 1)),
        reason: message.substring(0, 40),
      );
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
