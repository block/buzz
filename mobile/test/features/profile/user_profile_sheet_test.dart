import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';

void main() {
  testWidgets('does not label missing profile presence as offline', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          currentPubkeyProvider.overrideWithValue('self'),
          relaySessionProvider.overrideWith(() => _FakeRelaySessionNotifier()),
          presenceCacheProvider.overrideWith(
            () => _FakePresenceCacheNotifier(const {}),
          ),
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(const {
              'fable': UserProfile(pubkey: 'fable', displayName: 'Fable'),
            }),
          ),
          userStatusCacheProvider.overrideWith(
            () => _FakeUserStatusCacheNotifier(),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const Scaffold(body: UserProfileSheet(pubkey: 'fable')),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Status unknown'), findsOneWidget);
    expect(find.text('Offline'), findsNothing);
  });
}

class _FakeRelaySessionNotifier extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];
}

class _FakePresenceCacheNotifier extends PresenceCacheNotifier {
  final Map<String, String> _presence;

  _FakePresenceCacheNotifier(this._presence);

  @override
  Map<String, String> build() => _presence;

  @override
  void track(List<String> pubkeys) {}
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;

  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;

  @override
  UserProfile? get(String pubkey) => _users[pubkey.toLowerCase()];

  @override
  void preload(List<String> pubkeys) {}
}

class _FakeUserStatusCacheNotifier extends UserStatusCacheNotifier {
  @override
  Map<String, UserStatus?> build() => const {};

  @override
  void track(List<String> pubkeys) {}
}
