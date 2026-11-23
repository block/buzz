part of 'channels_provider_test.dart';

/// Read state that counts marker lookups. A highlight pass looks markers up
/// only while it reads a channel's events, so a pass served from the memo
/// makes none.
class _CountingReadState extends ReadStateState {
  _CountingReadState()
    : super(isReady: true, pubkey: 'me', contexts: const {}, version: 0);

  int lookups = 0;

  @override
  int? effectiveTimestamp(String contextId) {
    lookups++;
    return super.effectiveTimestamp(contextId);
  }
}

void _unreadRevisionCases() {
  test('replayed history and no-op changes keep cached highlights', () async {
    final base = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 100;
    final root = 'f' * 64;
    NostrEvent mention(String id, int at) => NostrEvent(
      id: id,
      pubkey: 'alice',
      createdAt: base + at,
      kind: 9,
      tags: const [
        ['h', _channelA],
        ['p', 'me'],
      ],
      content: 'hello',
      sig: 'sig',
    );
    final reply = NostrEvent(
      id: 'followed-reply',
      pubkey: 'alice',
      createdAt: base + 55,
      kind: 9,
      tags: [
        ['h', _channelA],
        ['e', root, '', 'root'],
        ['e', root, '', 'reply'],
      ],
      content: 'reply',
      sig: 'sig',
    );
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, 'me'), _membership(_channelB, 'me')],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'other'),
      ],
    );
    final follows = _TestFollows();
    final container = _buildContainer(session: session, follows: follows);
    addTearDown(container.dispose);
    final listed = await container.read(channelsProvider.future);
    await _settle();
    final notifier = container.read(channelsProvider.notifier);
    follows.choose(followed: {root});
    final first = mention('first-mention', 50);
    session.emit(first);
    session.emit(reply);
    await _settle();

    final readState = _CountingReadState();
    int revision([String channelId = _channelA]) =>
        (notifier.observedUnreadEventsByChannel as ObservedUnreadSnapshot)
            .revisionOf(channelId);
    int pass() {
      readState.lookups = 0;
      final highlights = computeChannelListHighlights(
        channels: listed,
        readState: readState,
        channelsNotifier: notifier,
      );
      expect(highlights.attention, {_channelA});
      return readState.lookups;
    }

    expect(pass(), greaterThan(0), reason: 'the first pass reads the events');
    expect(pass(), 0, reason: 'unchanged inputs');
    final held = revision();
    final heldEmpty = revision(_channelB);
    Set<String> ids() =>
        notifier.observedUnreadEventsByChannel[_channelA]!.keys.toSet();
    expect(ids(), {first.id, reply.id});

    // Refresh fetches history the registry already holds.
    session.recentMessages = [first, reply];
    await notifier.refresh();
    await notifier.waitForUnreadCatchUp();
    expect(ids(), {first.id, reply.id});
    expect(revision(), held, reason: 'replayed catch-up');
    expect(pass(), 0, reason: 'replayed catch-up');

    // So does a live redelivery after reconnect.
    session.emit(first);
    await _settle();
    expect(revision(), held, reason: 'replayed live event');
    expect(pass(), 0, reason: 'replayed live event');

    // An unfollow that removes no held reply, and a clear of a channel that
    // holds nothing, change no events either.
    follows.choose(followed: {root}, unfollowed: {'e' * 64});
    await _settle();
    notifier.clearObservedUnreadForChannel(_channelB);
    expect(revision(), held, reason: 'unfollow with nothing to drop');
    expect(revision(_channelB), heldEmpty, reason: 'clearing an empty channel');
    expect(pass(), 0, reason: 'no-op unfollow and clear');

    // Each real change still invalidates the channel.
    session.emit(mention('second-mention', 60));
    await _settle();
    final afterInsert = revision();
    expect(afterInsert, greaterThan(held), reason: 'a new event');
    expect(pass(), greaterThan(0), reason: 'a new event');

    follows.choose(unfollowed: {root});
    await _settle();
    expect(ids(), isNot(contains(reply.id)));
    final afterUnfollow = revision();
    expect(afterUnfollow, greaterThan(afterInsert), reason: 'a dropped reply');
    expect(pass(), greaterThan(0), reason: 'a dropped reply');

    notifier.clearObservedUnreadForChannel(_channelA);
    expect(revision(), greaterThan(afterUnfollow), reason: 'a real clear');
  });
}
