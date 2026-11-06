import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_unread.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/channels/unread_badge/unread_badge_provider.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The app badge is set only when the channel list shows an unread row, and
/// the list never shows an unread row the badge leaves out.
void main() {
  const eventAt = 20;
  const channelTypes = ['stream', 'forum', 'dm', 'not-yet-rendered'];
  const memberships = ['member', 'non-member', 'archived'];
  const eventShapes = [
    'none',
    'top-level',
    'reply',
    'mention',
    'mention-reply',
  ];
  const readStates = ['unseeded', 'read-before', 'read-after', 'forced'];

  var badgedCases = 0;
  var quietCases = 0;

  for (final type in channelTypes) {
    for (final membership in memberships) {
      for (final shape in eventShapes) {
        for (final read in readStates) {
          test('$type, $membership, $shape, $read', () async {
            final channel = Channel(
              id: 'ch',
              name: 'ch',
              channelType: type,
              visibility: 'open',
              description: '',
              createdBy: 'owner',
              createdAt: DateTime.utc(2026),
              memberCount: 2,
              isMember: membership != 'non-member',
              archivedAt: membership == 'archived' ? DateTime.utc(2026) : null,
              lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
                eventAt * 1000,
                isUtc: true,
              ),
            );
            final events = [
              if (shape != 'none')
                makeObservedUnreadEvent(
                  id: 'event',
                  createdAt: eventAt,
                  rootId: shape.endsWith('reply') ? 'root' : null,
                  highPriority: shape.startsWith('mention'),
                  channelType: type,
                  isThreadedReply: shape.endsWith('reply'),
                ),
            ];
            final readState = ReadStateState(
              isReady: true,
              pubkey: 'me',
              contexts: switch (read) {
                'read-before' => {'ch': eventAt - 10},
                'read-after' => {'ch': eventAt + 10},
                _ => const {},
              },
              version: 1,
              forcedUnreadContexts: read == 'forced'
                  ? const {'ch': 'ch'}
                  : const {},
            );
            final notifier = _Channels([channel], {'ch': events});
            final container = ProviderContainer(
              overrides: [
                channelsProvider.overrideWith(() => notifier),
                readStateProvider.overrideWith(() => _ReadState(readState)),
              ],
            );
            addTearDown(container.dispose);

            await container.read(channelsProvider.future);
            final badge = container.read(unreadBadgeProvider);
            final badged =
                badge.highPriorityCount + badge.generalUnreadCount > 0;

            final highlights = computeChannelListHighlights(
              channels: [channel].where((c) => c.showsInChannelList),
              readState: readState,
              channelsNotifier: notifier,
            );
            final rowUnread =
                highlights.unread.isNotEmpty || highlights.attention.isNotEmpty;

            badged ? badgedCases++ : quietCases++;
            expect(
              badged,
              rowUnread,
              reason: badged
                  ? 'badge with no unread row'
                  : 'unread row with no badge',
            );
          });
        }
      }
    }
  }

  tearDownAll(() {
    expect(badgedCases, greaterThan(0));
    expect(quietCases, greaterThan(0));
  });
}

class _Channels extends ChannelsNotifier {
  _Channels(this._channels, Map<String, List<ObservedUnreadEvent>> events)
    : _events = {
        for (final MapEntry(:key, :value) in events.entries)
          if (value.isNotEmpty)
            key: {for (final event in value) event.id: event},
      };

  final List<Channel> _channels;
  final Map<String, Map<String, ObservedUnreadEvent>> _events;

  @override
  Future<List<Channel>> build() async => _channels;

  @override
  Map<String, int> get latestObservedByChannel => {
    for (final MapEntry(:key, :value) in _events.entries)
      key: value.values
          .map((event) => event.createdAt)
          .reduce((a, b) => a > b ? a : b),
  };

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => _events;
}

class _ReadState extends ReadStateNotifier {
  _ReadState(this._state);

  final ReadStateState _state;

  @override
  ReadStateState build() => _state;
}
