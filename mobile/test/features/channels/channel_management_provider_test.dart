import 'package:flutter_test/flutter_test.dart';
import 'package:sprout_mobile/features/channels/channel_management_provider.dart';
import 'package:sprout_mobile/shared/relay/relay.dart';

/// Tests for [channelDetailsFromEvent].
///
/// The function maps a kind:39000 metadata event to [ChannelDetails], and is
/// the source of truth for the merge that [Channel.mergeDetails] performs in
/// the channel detail view. Anything `ChannelData.fromEvent` parses that's
/// also exposed on `ChannelDetails` MUST be propagated here — otherwise
/// `mergeDetails` silently clears that state on the merged Channel.
void main() {
  test('propagates archived state from kind:39000 archived tag', () {
    // Regression: previously this mapping ignored the `archived` tag, so
    // `Channel.mergeDetails` would clear the archived flag the list provider
    // had set, and the detail screen would show compose/manage actions for
    // expired/archived TTL channels.
    final details = channelDetailsFromEvent(
      NostrEvent(
        id: 'meta-1',
        pubkey: 'creator',
        createdAt: 1700000000,
        kind: 39000,
        tags: const [
          ['d', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
          ['name', 'expired-ttl'],
          ['t', 'stream'],
          ['public'],
          ['ttl', '86400'],
          ['archived', 'true'],
        ],
        content: '',
        sig: 'sig',
      ),
    );

    expect(details.archivedAt, isNotNull);
    expect(details.ttlSeconds, 86400);
  });

  test('omits archivedAt when no archived tag is present', () {
    final details = channelDetailsFromEvent(
      NostrEvent(
        id: 'meta-1',
        pubkey: 'creator',
        createdAt: 1700000000,
        kind: 39000,
        tags: const [
          ['d', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
          ['name', 'active'],
          ['t', 'stream'],
          ['public'],
        ],
        content: '',
        sig: 'sig',
      ),
    );

    expect(details.archivedAt, isNull);
    expect(details.ttlSeconds, isNull);
  });

  test('propagates ttl_deadline tag', () {
    final details = channelDetailsFromEvent(
      NostrEvent(
        id: 'meta-1',
        pubkey: 'creator',
        createdAt: 1700000000,
        kind: 39000,
        tags: const [
          ['d', 'c8c629ae-d35c-44fa-bc39-f6c1816756cc'],
          ['name', 'with-deadline'],
          ['t', 'stream'],
          ['public'],
          ['ttl', '86400'],
          ['ttl_deadline', '2026-05-14T19:54:06.989151+00:00'],
        ],
        content: '',
        sig: 'sig',
      ),
    );

    expect(details.ttlSeconds, 86400);
    expect(details.ttlDeadline, isNotNull);
    expect(details.ttlDeadline!.isUtc, isTrue);
  });
}
