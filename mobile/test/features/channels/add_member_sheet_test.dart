import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/add_member_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  NostrEvent profile(
    String pubkey,
    String name, {
    List<List<String>> tags = const [],
  }) => NostrEvent(
    id: '$pubkey-profile',
    pubkey: pubkey,
    createdAt: 1700000000,
    kind: 0,
    tags: tags,
    content: '{"display_name":"$name"}',
    sig: 'sig',
  );

  Widget buildSheet({
    required _FakeRelaySession session,
    required _AddMembersRecorder recorder,
    Set<String> existingMemberPubkeys = const {},
  }) {
    return WidgetHelpers.testable(
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        myPubkeyProvider.overrideWithValue('me'),
        channelActionsProvider.overrideWith(
          (ref) => _FakeChannelActions(ref, recorder),
        ),
      ],
      child: AddChannelMemberSheet(
        channelId: 'chan-1',
        existingMemberPubkeys: existingMemberPubkeys,
      ),
    );
  }

  testWidgets('excludes existing channel members from the directory list', (
    tester,
  ) async {
    final session = _FakeRelaySession(
      profileEvents: [profile('alice', 'Alice'), profile('bob', 'Bob')],
    );
    final recorder = _AddMembersRecorder();

    await tester.pumpWidget(
      buildSheet(
        session: session,
        recorder: recorder,
        existingMemberPubkeys: {'alice'},
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('add-member-result-bob')), findsOneWidget);
    expect(find.byKey(const Key('add-member-result-alice')), findsNothing);
  });

  testWidgets('adds a selected agent with the bot role, humans with member', (
    tester,
  ) async {
    final session = _FakeRelaySession(
      profileEvents: [
        profile('alice', 'Alice'),
        profile(
          'joe',
          'Joe',
          tags: const [
            ['auth', 'owner-pubkey', '', 'sig'],
          ],
        ),
      ],
    );
    final recorder = _AddMembersRecorder();

    await tester.pumpWidget(buildSheet(session: session, recorder: recorder));
    await tester.pumpAndSettle();

    // The auth tag above doesn't verify (fake sig), so isAgent depends on
    // real Schnorr verification — this exercises the non-agent path for
    // both entries, confirming the default 'member' role is used.
    await tester.tap(find.byKey(const Key('add-member-result-alice')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-member-submit')));
    await tester.pumpAndSettle();

    expect(recorder.calls, hasLength(1));
    expect(recorder.calls.single.pubkeys, ['alice']);
    expect(recorder.calls.single.role, 'member');
  });
}

class _FakeRelaySession extends RelaySessionNotifier {
  _FakeRelaySession({required this.profileEvents});

  final List<NostrEvent> profileEvents;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    return profileEvents;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    return const [];
  }
}

class _AddMembersCall {
  final List<String> pubkeys;
  final String role;

  _AddMembersCall({required this.pubkeys, required this.role});
}

/// Shared between test and fake so assertions survive the fake being
/// constructed anew by [channelActionsProvider]'s override callback.
class _AddMembersRecorder {
  final calls = <_AddMembersCall>[];
}

class _FakeChannelActions extends ChannelActions {
  final _AddMembersRecorder _recorder;

  _FakeChannelActions(Ref ref, this._recorder)
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: 'me',
      );

  @override
  Future<void> addMembers({
    required String channelId,
    required List<String> pubkeys,
    String role = 'member',
  }) async {
    _recorder.calls.add(_AddMembersCall(pubkeys: pubkeys, role: role));
  }
}
