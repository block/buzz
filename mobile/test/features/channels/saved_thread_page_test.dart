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

  setUp(() async {
    root = await Directory.systemTemp.createTemp('buzz-thread-cache');
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
        expect(await readSaved(), [oldReply, newReply]);

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
    relay.writerEvents = [head, edit];
    final replies = threadRepliesProvider(args);
    container.listen(replies, (_, _) {});
    expect(await container.read(replies.future), [reply]);
    await paint();

    final saved = await readSaved();
    expect(saved, [head, reply, edit]);
    expect(
      formatTimeline(
        saved!,
      ).singleWhere((event) => event.id == reply.id).content,
      'Edited reply',
    );
    expect(relay.writerFilters, hasLength(2));
    expect(relay.writerFilters.first.ids, [head.id]);
    expect(
      relay.writerFilters.last.tags['#e'],
      unorderedEquals([head.id, reply.id]),
    );
    for (final filter in relay.writerFilters) {
      expect(filter.tags['#h'], [args.channelId]);
    }
  });

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
