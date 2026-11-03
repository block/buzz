import 'package:buzz/features/agents/first_agent.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/remote_agents_fixture.dart';

final _dm = Channel(
  id: 'dm',
  name: '',
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: fakeOwner.public,
  createdAt: DateTime.utc(2026),
  memberCount: 2,
  isMember: true,
);

class _Actions implements ChannelActions {
  final opened = <List<String>>[];
  bool fail = false;

  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    opened.add(pubkeys);
    if (fail) throw StateError('Relay unavailable.');
    return _dm;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Sender implements SendMessage {
  final sent = <(String, String, List<String>?)>[];

  @override
  Future<void> call({
    required String channelId,
    required String content,
    String? parentEventId,
    String? rootEventId,
    List<String>? mentionPubkeys,
    Channel? channel,
    List<List<String>> mediaTags = const [],
  }) async => sent.add((channelId, content, mentionPubkeys));

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late FakeRegistry registry;
  late RelayFakeController controller;
  late _Actions actions;
  late _Sender sender;
  late ProviderContainer container;
  final community = Community.create(
    name: 'Test',
    relayUrl: 'wss://test.${HostedEnvironment.communitySuffix}',
    pubkey: fakeOwner.public,
    nsec: fakeOwner.nsec,
  );

  var current = community;

  setUp(() async {
    current = community;
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({});
    registry = FakeRegistry(bound: fakeOwner.public);
    await AccountIdentityStore().forSubject(fakeSubject).create(fakeOwner.nsec);
    controller = RelayFakeController(registry);
    actions = _Actions();
    sender = _Sender();
    container = ProviderContainer(
      overrides: [
        remoteAgentControllerProvider.overrideWithValue(controller),
        remoteAgentsEnabledProvider.overrideWithValue(true),
        activeCommunityProvider.overrideWith((ref) async => current),
        channelActionsProvider.overrideWithValue(actions),
        sendMessageProvider.overrideWithValue(sender),
      ],
    );
    await container.read(activeCommunityProvider.future);
    addTearDown(() {
      container.dispose();
      controller.dispose();
      controller.relayAuth.dispose();
      registry.dispose();
    });
  });

  test('the starter model is the first offered, at medium when supported', () {
    RemoteAgentModel model(String id, List<String> efforts) =>
        RemoteAgentModel(id: id, name: id, efforts: efforts);
    expect(starterModel([]), isNull);
    expect(
      starterModel([
        model('a', ['low', 'medium']),
        model('b', ['x']),
      ]),
      (model: 'a', effort: 'medium'),
    );
    expect(
      starterModel([
        model('a', ['low', 'high']),
      ]),
      (model: 'a', effort: 'low'),
    );
  });

  test('setup creates the agent, opens its DM and says hello once', () async {
    final setup = container.read(firstAgentSetupProvider.notifier);
    setup.pick(name: ' Nova ', picture: 'https://example.com/nova.png');
    await setup.start(community);

    final state = container.read(firstAgentSetupProvider);
    expect(state.error, isNull);
    expect(state.working, isFalse);
    expect(state.ready, same(_dm));
    expect(registry.bodies['/v3/beekeeper/register-agent'], {
      'agent_name': 'Nova',
      'agent_picture_url': 'https://example.com/nova.png',
      'idempotency_key': isA<String>(),
    });
    expect(registry.bodies['/v3/beekeeper/update-agent'], {
      'agent_pubkey': fakeAgent,
      'agent_instructions': starterAgentInstructions.trim(),
      'model_id': 'test-model',
      'reasoning_effort': 'low',
    });
    expect(actions.opened, [
      [fakeAgent],
    ]);
    final (channelId, content, mentions) = sender.sent.single;
    expect(channelId, 'dm');
    expect(content, starterAgentHello);
    expect(mentions, [fakeAgent]);
  });

  test('retrying after the agent exists only reopens its DM', () async {
    actions.fail = true;
    final setup = container.read(firstAgentSetupProvider.notifier);
    setup.pick(name: 'Nova');
    await setup.start(community);
    expect(container.read(firstAgentSetupProvider).error, isNotNull);
    expect(container.read(firstAgentSetupProvider).ready, isNull);

    actions.fail = false;
    await setup.start(community);
    expect(container.read(firstAgentSetupProvider).ready, same(_dm));
    expect(
      registry.calls.where((call) => call == 'register-agent'),
      hasLength(1),
    );
    expect(sender.sent, hasLength(1));
  });

  test('retrying a setup that failed midway resumes the same agent', () async {
    registry.failInstructions = true;
    final setup = container.read(firstAgentSetupProvider.notifier);
    setup.pick(name: 'Nova');
    await setup.start(community);
    expect(container.read(firstAgentSetupProvider).error, isNotNull);
    expect(actions.opened, isEmpty);

    registry.failInstructions = false;
    await setup.start(community);
    expect(container.read(firstAgentSetupProvider).ready, same(_dm));
    expect(
      registry.calls.where((call) => call == 'register-agent'),
      hasLength(1),
    );
    expect(actions.opened, [
      [fakeAgent],
    ]);
  });

  test('a setup belongs to the community it started in', () async {
    registry.rejectRegistration = true;
    final setup = container.read(firstAgentSetupProvider.notifier);
    setup.pick(name: 'Nova');
    await setup.start(community);
    expect(container.read(showFirstAgentProvider), isTrue);

    final other = Community.create(
      name: 'Other',
      relayUrl: 'wss://other.${HostedEnvironment.communitySuffix}',
      // Someone else's community: only the unfinished setup could show here.
      pubkey: 'f' * 64,
      nsec: fakeOwner.nsec,
    );
    current = other;
    container.invalidate(activeCommunityProvider);
    await container.read(activeCommunityProvider.future);
    expect(container.read(showFirstAgentProvider), isFalse);
  });

  test('a failed setup keeps the picks and reports why', () async {
    registry.rejectRegistration = true;
    final setup = container.read(firstAgentSetupProvider.notifier);
    setup.pick(name: 'Nova', picture: 'https://example.com/nova.png');
    await setup.start(community);
    final state = container.read(firstAgentSetupProvider);
    expect(state.working, isFalse);
    expect(state.error, isNotEmpty);
    expect(state.name, 'Nova');
    expect(state.picture, 'https://example.com/nova.png');
    expect(actions.opened, isEmpty);
  });

  test('DMs offers a first agent only once the registry shows none', () async {
    expect(container.read(showFirstAgentProvider), isFalse);
    await controller.run(controller.refresh);
    expect(container.read(showFirstAgentProvider), isTrue);

    registry.failListing = true;
    await controller.run(controller.refresh);
    expect(container.read(showFirstAgentProvider), isFalse);

    registry
      ..failListing = false
      ..listed = [
        {
          'agent_pubkey': fakeAgent,
          'agent_id': 'agent',
          'agent_name': 'Scout',
          'status': 2,
        },
      ];
    await controller.run(controller.refresh);
    expect(container.read(showFirstAgentProvider), isFalse);
  });

  test('the setup view opens by itself only once per account', () async {
    await controller.run(controller.refresh);
    expect(await claimFirstAgentAutoOpen(controller), isTrue);
    expect(await claimFirstAgentAutoOpen(controller), isFalse);
  });
}
