import 'package:buzz/features/channels/channel_mutes/channel_mutes_manager.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_manager.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('local star changes notify listeners immediately', () async {
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    var changes = 0;
    final manager = ChannelStarsManager(
      pubkey: keys.public,
      prefs: prefs,
      crypto: ChannelStarsCrypto(keys.nsec, keys.public),
      relaySession: null,
      signedEventRelay: null,
      remoteEnabled: false,
      onChanged: () => changes += 1,
    );
    addTearDown(manager.dispose);

    manager.starChannel('general');
    expect(manager.store.channels['general']?.starred, isTrue);
    expect(changes, 1);

    manager.unstarChannel('general');
    expect(manager.store.channels['general']?.starred, isFalse);
    expect(changes, 2);
  });

  test('local mute changes notify listeners immediately', () async {
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    var changes = 0;
    final manager = ChannelMutesManager(
      pubkey: keys.public,
      prefs: prefs,
      crypto: ChannelMutesCrypto(keys.nsec, keys.public),
      relaySession: null,
      signedEventRelay: null,
      remoteEnabled: false,
      onChanged: () => changes += 1,
    );
    addTearDown(manager.dispose);

    manager.muteChannel('general');
    expect(manager.store.channels['general']?.muted, isTrue);
    expect(changes, 1);

    manager.unmuteChannel('general');
    expect(manager.store.channels['general']?.muted, isFalse);
    expect(changes, 2);
  });
}
