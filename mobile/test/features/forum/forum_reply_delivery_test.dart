import 'dart:async';

import 'package:buzz/features/forum/forum_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test('a forum comment p-tags the post author it answers', () async {
    // Forum channels are mention-eligible for agents, and an agent's
    // `require_mention` subscription is a `#p` REQ filter — an untagged
    // comment never reaches the agent being replied to.
    final session = _PendingPublishRelaySession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container
        .read(relayConfigProvider.notifier)
        .update(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
        );

    final delivery = ForumEventDelivery.capture(container);
    final result = delivery.createReply(
      channelId: _channelId,
      parentEventId: _parentEventId,
      parentAuthorPubkey: _agentPubkey,
      content: 'thanks',
    );
    await session.published;
    session.accept();
    await result;

    // Dart lists compare by identity, so matchers are required here.
    expect(
      session.event.tags,
      containsAll([
        // Marked, not bare: a bare `p` tag here is byte-identical to a typed
        // @mention, which forces the receiver to fetch the parent just to tell
        // them apart. Relay tag filters match only the second element, so the
        // marker cannot affect the agent's `#p` delivery.
        equals(['p', _agentPubkey, '', 'reply']),
        equals(['e', _parentEventId, '', 'reply']),
      ]),
    );
  });

  test('a forum comment without a parent author adds no extra p-tag', () async {
    final session = _PendingPublishRelaySession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container
        .read(relayConfigProvider.notifier)
        .update(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
        );

    final delivery = ForumEventDelivery.capture(container);
    final result = delivery.createReply(
      channelId: _channelId,
      parentEventId: _parentEventId,
      content: 'thanks',
    );
    await session.published;
    session.accept();
    await result;

    expect(session.event.tags.where((tag) => tag.first == 'p'), isEmpty);
  });

  test(
    'a forum comment marks a typed mention apart from the post author',
    () async {
      // Same two roles as the channel builder, and the same reason they cannot be
      // told apart as bare tags: one pubkey was typed, the other wrote the post.
      final session = _PendingPublishRelaySession();
      final typed = 'd' * 64;
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => session)],
      );
      addTearDown(container.dispose);
      container
          .read(relayConfigProvider.notifier)
          .update(
            baseUrl: 'https://relay.example',
            nsec: nostr.Keys.generate().nsec,
          );

      final delivery = ForumEventDelivery.capture(container);
      final result = delivery.createReply(
        channelId: _channelId,
        parentEventId: _parentEventId,
        parentAuthorPubkey: _agentPubkey,
        content: 'thanks @typed',
        mentionPubkeys: [typed],
      );
      await session.published;
      session.accept();
      await result;

      expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
        ['p', typed, '', 'mention'],
        ['p', _agentPubkey, '', 'reply'],
      ]);
    },
  );

  test('a typed post author is tagged once, as a mention', () async {
    final session = _PendingPublishRelaySession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container
        .read(relayConfigProvider.notifier)
        .update(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
        );

    final delivery = ForumEventDelivery.capture(container);
    final result = delivery.createReply(
      channelId: _channelId,
      parentEventId: _parentEventId,
      parentAuthorPubkey: _agentPubkey,
      content: 'thanks @agent',
      mentionPubkeys: const [_agentPubkey],
    );
    await session.published;
    session.accept();
    await result;

    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
      ['p', _agentPubkey, '', 'mention'],
    ]);
  });

  test('a malformed post author adds no addressing tag', () async {
    final session = _PendingPublishRelaySession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container
        .read(relayConfigProvider.notifier)
        .update(
          baseUrl: 'https://relay.example',
          nsec: nostr.Keys.generate().nsec,
        );

    final delivery = ForumEventDelivery.capture(container);
    final result = delivery.createReply(
      channelId: _channelId,
      parentEventId: _parentEventId,
      parentAuthorPubkey: 'not-a-pubkey',
      content: 'thanks',
    );
    await session.published;
    session.accept();
    await result;

    expect(session.event.tags.where((tag) => tag.first == 'p'), isEmpty);
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _parentEventId =
    'fdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfd';
const _agentPubkey =
    'aeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeae';

class _PendingPublishRelaySession extends RelaySessionNotifier {
  final Completer<NostrEvent> _result = Completer<NostrEvent>();
  final Completer<void> _published = Completer<void>();
  late NostrEvent event;

  Future<void> get published => _published.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    this.event = event;
    _published.complete();
    return _result.future;
  }

  void accept() => _result.complete(event);
}
