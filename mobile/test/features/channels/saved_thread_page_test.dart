import 'dart:async';
import 'dart:collection';
import 'dart:io';

import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/features/channels/saved_channel_page.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/channels/saved_message_store.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/relay/relay_author_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/thread_ids.dart';

const _scope = (communityId: 'cache-test-community', pubkey: 'viewer');
const _args = ThreadRepliesArgs(channelId: 'channel', rootId: threadIdRoot);
const _otherArgs = ThreadRepliesArgs(channelId: 'other', rootId: threadIdRoot);

NostrEvent _event(
  String content, {
  ThreadRepliesArgs args = _args,
  int kind = EventKind.streamMessage,
  String? deletionTarget,
  bool isRoot = false,
}) => NostrEvent.fromJson(
  signNostrEvent(
    secretKey:
        '0000000000000000000000000000000000000000000000000000000000000001',
    kind: kind,
    content: content,
    tags: [
      ['h', args.channelId],
      if (deletionTarget != null)
        ['e', deletionTarget]
      else if (!isRoot)
        ['e', args.rootId, '', 'reply'],
    ],
  ).toMap(),
);

class _Relay extends RelaySessionNotifier {
  final threadQueries = Queue<Future<List<NostrEvent>>>();
  List<NostrEvent> writerEvents = const [];
  final writerFilters = <NostrFilter>[];
  final channelQuery = Completer<List<NostrEvent>>();
  final subscribed = Completer<void>();
  late void Function(NostrEvent) emit;
  late void Function(String) closeChannel;

  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  void connect() => state = const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters.first.extensions.containsKey('thread_cursor')) {
      return threadQueries.removeFirst();
    }
    if (filters.every(
      (filter) => filter.extensions['consistency'] == 'strong',
    )) {
      writerFilters.addAll(filters);
      return writerEvents;
    }
    return channelQuery.future;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    emit = onEvent;
    closeChannel = onClosed!;
    subscribed.complete();
    return () {};
  }
}

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();
  late Directory root;
  late _Relay relay;
  late ProviderContainer container;
  late SavedMessageStore store;

  void openCacheSession() {
    relay = _Relay();
    container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(() => relay),
        relayAuthorProvider.overrideWith((ref, url) => null),
        savedMessageScopeProvider.overrideWithValue(_scope),
        savedMessageRootProvider.overrideWith((ref) async => root),
      ],
    );
    store = container.read(savedMessageStoreProvider);
  }

  setUp(() async {
    root = await Directory.systemTemp.createTemp('buzz-thread-cache');
    openCacheSession();
  });

  tearDown(() async {
    container.dispose();
    await root.delete(recursive: true);
  });

  Future<void> paint() async {
    await Future<void>.delayed(Duration.zero);
    binding.handleBeginFrame(null);
    binding.handleDrawFrame();
    await Future<void>.delayed(Duration.zero);
  }

  Future<List<NostrEvent>?> waitForSaved(
    SavedMessagePage page,
    Iterable<NostrEvent> expected,
  ) async {
    final ids = expected.map((event) => event.id).toSet();
    final elapsed = Stopwatch()..start();
    List<NostrEvent>? saved;
    do {
      saved = await store.read(store.capture(_scope, page));
      if (saved != null &&
          saved.length == ids.length &&
          saved.every((event) => ids.contains(event.id))) {
        return saved;
      }
      // Yield to the write worker while observing the durable result.
      await Future<void>.delayed(const Duration(milliseconds: 1));
    } while (elapsed.elapsed < const Duration(seconds: 2));
    return saved;
  }

  for (final lateRefusal in [false, true]) {
    test(
      'an invalidated thread scan cannot ${lateRefusal ? 'purge' : 'overwrite'} the newer page',
      () async {
        final retired = Completer<List<NostrEvent>>();
        final oldReply = _event('Old reply');
        final newReply = _event('New reply');
        relay.threadQueries.addAll([
          retired.future,
          Future.value([oldReply, newReply]),
        ]);

        final replies = threadRepliesProvider(_args);
        container.listen(replies, (_, _) {});
        expect(relay.threadQueries, hasLength(1));
        container.invalidate(replies);
        expect(await container.read(replies.future), [oldReply, newReply]);
        await paint();

        Future<List<NostrEvent>?> readSaved() =>
            store.read(store.capture(_scope, _args.savedPage));
        expect(await waitForSaved(_args.savedPage, [oldReply, newReply]), [
          oldReply,
          newReply,
        ]);

        final refusal = RelayException(403, 'restricted: not a channel member');
        if (lateRefusal) {
          retired.completeError(refusal);
        } else {
          retired.complete([oldReply]);
        }
        await paint();
        expect(await readSaved(), [oldReply, newReply]);

        // A refusal from the current scan still purges the page.
        relay.threadQueries.add(
          Future<List<NostrEvent>>.microtask(() => throw refusal),
        );
        container.invalidate(replies);
        await expectLater(
          container.read(replies.future),
          throwsA(same(refusal)),
        );
        expect(await readSaved(), isNull);
      },
    );
  }

  test('a new scan saves the signed root and edits from the writer', () async {
    final head = _event('Older thread head', isRoot: true);
    final args = ThreadRepliesArgs(channelId: _args.channelId, rootId: head.id);
    final reply = _event('Original reply', args: args);
    final edit = _event(
      'Edited reply',
      args: args,
      kind: EventKind.streamMessageEdit,
      deletionTarget: reply.id,
    );
    Future<List<NostrEvent>?> readSaved() =>
        store.read(store.capture(_scope, args.savedPage));

    await store.save(store.capture(_scope, args.savedPage), [
      reply,
    ], relayAuthor: null);
    container.listen(channelMessagesProvider(args.channelId), (_, _) {});
    relay.connect();
    await relay.subscribed.future;
    relay.emit(edit);
    expect(await readSaved(), isNull);

    relay.threadQueries.add(Future.value([reply]));
    relay.writerEvents = [head, reply, edit];
    final replies = threadRepliesProvider(args);
    final openThread = container.listen(replies, (_, _) {});
    expect(await container.read(replies.future), [reply]);
    await paint();

    final saved = await waitForSaved(args.savedPage, [head, reply, edit]);
    expect(relay.writerFilters, hasLength(2));
    expect(saved, [head, reply, edit]);
    expect(
      formatTimeline(
        saved!,
      ).singleWhere((event) => event.id == reply.id).content,
      'Edited reply',
    );
    expect(relay.writerFilters, hasLength(2));
    expect(relay.writerFilters.first.ids, unorderedEquals([head.id, reply.id]));
    expect(relay.writerFilters.last.tags['#e'], [reply.id]);
    for (final filter in relay.writerFilters) {
      expect(filter.tags['#h'], [args.channelId]);
    }

    final laterReply = _event('Arrived after opening', args: args);
    relay.threadQueries.add(Future.value([reply, laterReply]));
    container.invalidate(replies);
    expect(await container.read(replies.future), [reply, laterReply]);
    await paint();
    expect(relay.writerFilters, hasLength(2));
    expect(await readSaved(), saved);

    openThread.close();
    await container.pump();
    relay.writerEvents = [head, reply, edit];
    relay.threadQueries.add(Future.value([reply, laterReply]));
    final reopenedThread = container.listen(replies, (_, _) {});
    expect(await container.read(replies.future), [reply, laterReply]);
    await paint();
    expect(
      await waitForSaved(args.savedPage, [head, reply, laterReply, edit]),
      unorderedEquals([head, reply, laterReply, edit]),
    );
    expect(relay.writerFilters, hasLength(4));
    expect(relay.writerFilters.last.tags['#e'], [reply.id]);

    // The writer response is still untrusted. A forged edit cannot make the
    // already superseded original eligible for the saved page.
    final forged = NostrEvent.fromJson({...edit.toJson(), 'content': 'Forged'});
    reopenedThread.close();
    await container.pump();
    relay.writerEvents = [head, reply, forged];
    relay.threadQueries.add(Future.value([reply, laterReply]));
    container.listen(replies, (_, _) {});
    expect(await container.read(replies.future), [reply, laterReply]);
    await paint();
    expect(
      await waitForSaved(args.savedPage, [head, reply, laterReply, edit]),
      unorderedEquals([head, reply, laterReply, edit]),
    );
  });

  test(
    'a thread with a loaded root and no observed edits needs no extra query',
    () async {
      final head = _event('Loaded thread head', isRoot: true);
      final args = ThreadRepliesArgs(
        channelId: _args.channelId,
        rootId: head.id,
      );
      final reply = _event('Original reply', args: args);
      final channel = channelMessagesProvider(args.channelId);
      container.listen(channel, (_, _) {});
      relay.connect();
      await relay.subscribed.future;
      relay.emit(head);
      expect(container.read(channel).value, contains(head));
      relay.threadQueries.add(Future.value([reply]));
      final replies = threadRepliesProvider(args);
      final opened = container.listen(replies, (_, _) {});
      expect(await container.read(replies.future), [reply]);
      await paint();
      expect(relay.writerFilters, isEmpty);
      expect(await waitForSaved(args.savedPage, [head, reply]), [head, reply]);

      opened.close();
      await container.pump();
      final later = _event('New reply', args: args);
      relay.threadQueries.add(Future.value([reply, later]));
      container.listen(replies, (_, _) {});
      expect(await container.read(replies.future), [reply, later]);
      await paint();
      expect(await waitForSaved(args.savedPage, [head, reply, later]), [
        head,
        reply,
        later,
      ]);
      expect(relay.writerFilters, isEmpty);
    },
  );

  for (final prunedBeforeRestart in [false, true]) {
    test(
      'a replica cannot restore the original after restart (pruned: $prunedBeforeRestart)',
      () async {
        final head = _event('Older head', isRoot: true);
        final args = ThreadRepliesArgs(
          channelId: _args.channelId,
          rootId: head.id,
        );
        final reply = _event('Original', args: args);
        final edit = _event(
          'Revised',
          args: args,
          kind: EventKind.streamMessageEdit,
          deletionTarget: reply.id,
        );
        final ticket = store.capture(_scope, args.savedPage);
        await store.save(
          ticket,
          prunedBeforeRestart ? [reply] : [head, reply, edit],
          relayAuthor: null,
        );
        if (prunedBeforeRestart) {
          store.removeChannelEvents(_scope, args.channelId, {
            reply.id,
          }, edited: true);
          expect(
            await store.read(store.capture(_scope, args.savedPage)),
            isNull,
          );
        }
        container.dispose();
        openCacheSession();

        relay.threadQueries.add(Future.value([reply]));
        relay.writerEvents = [head, reply, edit];
        final provider = threadRepliesProvider(args);
        container.listen(provider, (_, _) {});
        expect(await container.read(provider.future), [reply]);
        await paint();
        expect(await waitForSaved(args.savedPage, [head, reply, edit]), [
          head,
          reply,
          edit,
        ]);
        expect(relay.writerFilters.last.tags['#e'], contains(reply.id));
      },
    );
  }

  test(
    'a pruned deletion survives restart and a later page replacement',
    () async {
      final head = _event('Saved head', isRoot: true);
      final args = ThreadRepliesArgs(
        channelId: _args.channelId,
        rootId: head.id,
      );
      final deleted = _event('Deleted reply', args: args);
      final newer = _event('New reply', args: args);
      await store.save(store.capture(_scope, args.savedPage), [
        head,
        deleted,
      ], relayAuthor: null);
      store.removeChannelEvents(_scope, args.channelId, {deleted.id});
      expect(
        (await store.inspectThread(
          store.capture(_scope, args.savedPage),
        ))?.pruned,
        {deleted.id: false},
      );
      container.dispose();
      openCacheSession();
      final channel = channelMessagesProvider(args.channelId);
      container.listen(channel, (_, _) {});
      relay.connect();
      await relay.subscribed.future;
      relay.emit(head);
      relay.threadQueries.add(Future.value([deleted, newer]));
      final replies = threadRepliesProvider(args);
      container.listen(replies, (_, _) {});
      await container.read(replies.future);
      await paint();
      expect(await waitForSaved(args.savedPage, [head, newer]), [head, newer]);
      expect(relay.writerFilters, isEmpty);
      expect(
        (await store.inspectThread(
          store.capture(_scope, args.savedPage),
        ))?.pruned,
        {deleted.id: false},
      );
      await store.removeChannel(_scope, args.channelId);
      expect(
        await store.inspectThread(store.capture(_scope, args.savedPage)),
        isNull,
      );
    },
  );

  test(
    'an invalid edit cannot replace a pruned thread after restart',
    () async {
      final head = _event('Older head', isRoot: true);
      final args = ThreadRepliesArgs(
        channelId: _args.channelId,
        rootId: head.id,
      );
      final reply = _event('Original', args: args);
      final edit = _event(
        'Revised',
        args: args,
        kind: EventKind.streamMessageEdit,
        deletionTarget: reply.id,
      );
      await store.save(store.capture(_scope, args.savedPage), [
        reply,
      ], relayAuthor: null);
      store.removeChannelEvents(_scope, args.channelId, {
        reply.id,
      }, edited: true);
      final prunedPage = await store.inspectThread(
        store.capture(_scope, args.savedPage),
      );
      expect(prunedPage?.events, isEmpty);
      expect(prunedPage?.pruned, {reply.id: true});
      container.dispose();
      openCacheSession();

      final ticket = store.capture(_scope, args.savedPage);
      final forged = NostrEvent.fromJson({
        ...edit.toJson(),
        'content': 'Forged',
      });
      await store.save(ticket, [head, reply, forged], relayAuthor: null);
      expect(await store.read(ticket), isNull);
      expect((await store.inspectThread(ticket))?.pruned, {reply.id: true});
    },
  );

  test(
    'an empty thread result cannot clear confirmed channel refusal',
    () async {
      final channel = channelMessagesProvider(_args.channelId);
      container.listen(channel, (_, _) {});
      relay.connect();
      await relay.subscribed.future;
      final response = Completer<List<NostrEvent>>();
      relay.threadQueries.add(response.future);
      final replies = threadRepliesProvider(_args);
      container.listen(replies, (_, _) {});
      final pending = container.read(replies.future);
      relay.closeChannel('restricted: not a channel member');
      expect(container.read(channel).error, isNotNull);
      response.complete([]);
      expect(await pending, isEmpty);
      await paint();
      expect(container.read(channel).error, isNotNull);
      expect(relay.writerFilters, isEmpty);
      expect(await store.read(store.capture(_scope, _args.savedPage)), isNull);
    },
  );

  test(
    'oversized threads keep complete relay history and skip the writer',
    () async {
      final old = _event('Previously saved reply');
      final ticket = store.capture(_scope, _args.savedPage);
      await store.save(ticket, [old], relayAuthor: null);
      final batch =
          [
            for (var index = 0; index < 201; index++)
              _event('Large thread reply $index'),
          ]..sort((a, b) {
            final time = a.createdAt.compareTo(b.createdAt);
            return time == 0 ? a.id.compareTo(b.id) : time;
          });
      relay.threadQueries.addAll([
        Future.value(batch.take(200).toList()),
        Future.value([batch.last]),
      ]);
      final replies = threadRepliesProvider(_args);
      container.listen(replies, (_, _) {});
      expect(await container.read(replies.future), batch);
      await paint();
      expect(relay.writerFilters, isEmpty);
      expect(await store.read(ticket), isNull);
    },
  );

  test('a live deletion retires an already displayed thread preview', () async {
    final removed = _event('Deleted while history is pending');
    final retained = _event('Same-channel neighbor');
    final unrelated = _event('Other channel', args: _otherArgs);
    final preview = savedThreadPreviewProvider(_args.savedPage);
    final otherPreview = savedThreadPreviewProvider(_otherArgs.savedPage);
    await store.save(store.capture(_scope, _args.savedPage), [
      removed,
      retained,
    ], relayAuthor: null);
    await store.save(store.capture(_scope, _otherArgs.savedPage), [
      unrelated,
    ], relayAuthor: null);
    container.listen(preview, (_, _) {});
    container.listen(otherPreview, (_, _) {});
    expect(await container.read(preview.future), [removed, retained]);
    expect(await container.read(otherPreview.future), [unrelated]);
    final unaffected = container.read(otherPreview);

    container.listen(channelMessagesProvider(_args.channelId), (_, _) {});
    relay.connect();
    await relay.subscribed.future;
    relay.emit(
      _event('', kind: EventKind.deletion, deletionTarget: removed.id),
    );
    final displayed = await container.read(preview.future);
    expect(await store.read(store.capture(_scope, _args.savedPage)), [
      retained,
    ]);
    expect(displayed, [retained]);
    expect(container.read(otherPreview), same(unaffected));
  });
}
