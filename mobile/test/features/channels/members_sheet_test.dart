import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/members_sheet.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';

const _channelId = 'test-channel';
const _selfPubkey =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _otherPubkey =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

final _channel = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'private',
  description: 'General discussion',
  createdBy: _selfPubkey,
  createdAt: DateTime(2025),
  memberCount: 1,
  isMember: true,
);

class _FakeChannelActions extends ChannelActions {
  _FakeChannelActions(Ref ref)
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: _selfPubkey,
      );

  final List<({String channelId, String pubkey, String role})> added = [];
  List<DirectoryUser> searchResults = const [];
  int searchCallCount = 0;
  String? lastSearchQuery;

  @override
  Future<List<DirectoryUser>> searchUsers(String query, {int limit = 8}) async {
    searchCallCount += 1;
    lastSearchQuery = query;
    // Yield so useFuture observes a non-synchronous completion.
    await Future<void>.delayed(Duration.zero);
    return searchResults
        .where(
          (user) =>
              (user.displayName ?? '').toLowerCase().contains(
                query.toLowerCase(),
              ) ||
              user.pubkey.toLowerCase().contains(query.toLowerCase()),
        )
        .take(limit)
        .toList();
  }

  @override
  Future<void> addMember({
    required String channelId,
    required String pubkey,
    String role = 'member',
  }) async {
    added.add((channelId: channelId, pubkey: pubkey, role: role));
  }
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;
  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;

  @override
  void preload(List<String> pubkeys) {}

  @override
  UserProfile? get(String pubkey) => _users[pubkey.toLowerCase()];
}

class _FakeUserStatusCacheNotifier extends UserStatusCacheNotifier {
  @override
  Map<String, UserStatus?> build() => {};

  @override
  void track(List<String> pubkeys) {}
}

void main() {
  late _FakeChannelActions actions;

  Widget buildSheet({required List<ChannelMember> members, Channel? channel}) {
    return ProviderScope(
      overrides: [
        channelMembersProvider(_channelId).overrideWith((ref) async => members),
        channelActionsProvider.overrideWith((ref) {
          actions = _FakeChannelActions(ref)
            ..searchResults = [
              const DirectoryUser(
                pubkey: _otherPubkey,
                displayName: 'Alice',
                nip05Handle: 'alice@example.com',
              ),
            ];
          return actions;
        }),
        userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
        userStatusCacheProvider.overrideWith(_FakeUserStatusCacheNotifier.new),
        relayClientProvider.overrideWithValue(
          RelayClient(baseUrl: 'http://localhost:3000'),
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: MembersSheet(
            channel: channel ?? _channel,
            currentPubkey: _selfPubkey,
          ),
        ),
      ),
    );
  }

  testWidgets('member can search and add people from the members sheet', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildSheet(
        members: [
          ChannelMember(
            pubkey: _selfPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
            displayName: 'You',
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Add people and agents'), findsOneWidget);
    // Section labels are uppercased by _SectionLabel.
    expect(find.text('PEOPLE — 1'), findsOneWidget);

    await tester.enterText(find.byType(TextField), 'ali');
    await tester.pump(); // process onChanged
    // Debounce is 250ms.
    await tester.pump(const Duration(milliseconds: 300));
    // Allow the search future microtask/delay to complete.
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    await tester.pumpAndSettle();

    expect(actions.searchCallCount, greaterThan(0));
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('NOT IN THIS CHANNEL'), findsOneWidget);

    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();

    expect(actions.added, hasLength(1));
    expect(actions.added.single.channelId, _channelId);
    expect(actions.added.single.pubkey, _otherPubkey);
    expect(actions.added.single.role, 'member');
  });

  testWidgets('dm channels hide the add-people search field', (tester) async {
    await tester.pumpWidget(
      buildSheet(
        members: [
          ChannelMember(
            pubkey: _selfPubkey,
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        channel: Channel(
          id: _channelId,
          name: 'DM',
          channelType: 'dm',
          visibility: 'private',
          description: '',
          createdBy: _selfPubkey,
          createdAt: DateTime(2025),
          memberCount: 2,
          isMember: true,
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Add people and agents'), findsNothing);
    expect(find.byType(TextField), findsNothing);
  });
}
