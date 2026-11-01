part of 'me_page.dart';

Future<(Channel, String)?> _showNewMeChat(
  BuildContext context,
  WidgetRef ref,
) async {
  final initialSync = ref.read(meSyncProvider);
  final unfinished = MeChatCreation.pending(initialSync);
  if (unfinished != null) {
    final resume = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Unfinished chat'),
        content: Text(
          'Resume “${unfinished.name}”, or discard its setup to start another chat. Any channel and messages already created are kept in Home or Solo.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Later'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Discard setup'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Resume'),
          ),
        ],
      ),
    );
    if (resume == null || !context.mounted) return null;
    if (!resume) {
      await unfinished.discard(initialSync);
      ref.invalidate(channelsProvider);
    }
  }
  if (!context.mounted) return null;
  final pendingDraft = MeChatCreation.pending(ref.read(meSyncProvider));
  if (pendingDraft == null || pendingDraft.initialMessage != null) {
    await Navigator.of(
      context,
    ).push<void>(MaterialPageRoute(builder: (_) => const _MeChatDraft()));
    return null;
  }
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final sync = ref.read(meSyncProvider);
    final pending = MeChatCreation.pending(sync);
    final id = pending?.id ?? const Uuid().v4();
    final messageMode = pending == null || pending.initialMessage != null;
    String name = pending?.initialMessage?.content ?? pending?.name ?? '';
    String? assistant = pending?.agent;
    String? project = pending?.project;
    String? error = pending == null ? null : 'Resume your unfinished chat.';
    var retry = pending != null;
    while (context.mounted) {
      sync.check();
      final directory = ref.read(meAssistantsProvider).asData?.value;
      final directoryRequest = ref.read(meAssistantsProvider).hasError
          ? ref.refresh(meAssistantsProvider.future)
          : ref.read(meAssistantsProvider.future);
      final presentationId = const Uuid().v4();
      var active = true;
      if (!context.mounted) return null;
      sync.check();
      if (!context.mounted) return null;
      final theme = utilitySurfaceThemeData(Theme.of(context));
      Map<Object?, Object?>? draft;
      ProviderSubscription<AsyncValue<String?>>? bestieSubscription;
      try {
        final presentation = const MethodChannel('buzz/profile_text_editor')
            .invokeMethod<Map<Object?, Object?>>('presentChat', {
              'title': 'New chat',
              'presentationId': presentationId,
              'assistantsLoading': directory == null,
              'initialValue': name,
              'placeholder': messageMode ? 'Message' : 'Chat name',
              'multiline': messageMode,
              'messageMode': messageMode,
              'brightness': theme.brightness.name,
              'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
              'containerBackgroundArgb': theme
                  .colorScheme
                  .surfaceContainerHighest
                  .toARGB32(),
              'containerCornerRadius': Radii.container,
              'selectedAssistant':
                  assistant ?? _availableBestie(ref, directory ?? []),
              if (retry && assistant != null)
                'savedAssistant': {
                  'id': assistant,
                  'name':
                      directory
                          ?.where((a) => a.pubkey == assistant)
                          .firstOrNull
                          ?.displayName ??
                      ref.read(userCacheProvider)[assistant]?.label ??
                      assistant,
                  'avatarUrl':
                      ref.read(userCacheProvider)[assistant]?.avatarUrl ?? '',
                },
              'selectedProject': project,
              'projects': [
                for (final group
                    in ref.read(meGroupsProvider).requireValue.groups)
                  {'id': group['id'], 'name': group['name']},
              ],
              'fieldsLocked': retry,
              'error': error,
              'assistants': _nativeMeAssistants(directory ?? []),
            });
        // Send updates only to this presentation; late results cannot populate
        // a reopened sheet or a different community's editor.
        Future<void> loadAssistants() async {
          try {
            final agents = await directoryRequest;
            if (!active || !context.mounted || sync.disposed) return;
            await const MethodChannel(
              'buzz/profile_text_editor',
            ).invokeMethod<void>('updateChatAssistants', {
              'presentationId': presentationId,
              'assistants': _nativeMeAssistants(agents),
              'defaultAssistant': _availableBestie(ref, agents),
            });
          } catch (_) {
            if (!active || !context.mounted || sync.disposed) return;
            try {
              await const MethodChannel(
                'buzz/profile_text_editor',
              ).invokeMethod<void>('updateChatAssistants', {
                'presentationId': presentationId,
                'assistants': <Object>[],
                'assistantError':
                    'Couldn’t load agents. Close this sheet and try again.',
              });
            } on PlatformException {
              // The native editor may already be dismissed.
            } on MissingPluginException {
              // Flutter fallback has no native editor.
            }
          }
        }

        loadAssistants();
        bestieSubscription = ref.listenManual(defaultAgentProvider, (_, next) {
          if (next.hasValue) loadAssistants();
        });
        draft = await presentation;
      } on MissingPluginException {
        break;
      } finally {
        active = false;
        bestieSubscription?.close();
      }
      if (draft == null || !context.mounted) return null;
      name = (draft['name'] as String? ?? '').trim();
      assistant = draft['assistant'] as String?;
      project = draft['project'] as String?;
      if (project == '') project = null;
      if (name.isEmpty || assistant == null) return null;
      retry = true;
      try {
        final created = await _createMeChat(
          ref,
          sync,
          id,
          messageMode ? startingMeChatTitle(name) : name,
          assistant,
          project,
          initialContent: messageMode ? name : null,
        );
        return (created, assistant);
      } catch (failure) {
        if (failure is MeRecoveryConflict && context.mounted) {
          await _offerMeRecovery(context, sync);
        }
        error =
            '${failure.toString().replaceFirst('Bad state: ', '')}\nRetry continues this same chat.';
      }
    }
  }
  if (!context.mounted) return null;
  return showBuzzModalBottomSheet<(Channel, String)>(
    context: context,
    title: 'New chat',
    isScrollControlled: true,
    requestFocus: true,
    builder: (_) => const _NewMeChat(),
  );
}

Future<Channel> _createMeChat(
  WidgetRef ref,
  MeSync sync,
  String channelId,
  String name,
  String assistant,
  String? project, {
  String? initialContent,
  List<List<String>> initialMediaTags = const [],
}) async {
  sync = _currentMeSync(ref, sync);
  final pending = MeChatCreation.pending(sync);
  final initialMessage = initialContent == null
      ? null
      : pending?.initialMessage ??
            NostrEvent.fromJson(
              signNostrEvent(
                kind: EventKind.streamMessage,
                content: initialContent,
                tags: [
                  ['h', channelId],
                  ['p', assistant],
                  ...initialMediaTags,
                ],
                secretKey: sync.privateHex,
              ).toMap(),
            );
  final intent = MeChatCreation(
    channelId,
    name,
    assistant,
    project: project,
    initialMessage: initialMessage,
  );
  await intent.save(sync);
  final actions = ref.read(channelActionsProvider);
  final preflight = await Future.wait<Object?>([
    isOwnedMeAssistant(sync, assistant),
    sync.head(),
    actions.findJoinedChannel(channelId),
  ]);
  final owned = preflight[0] as bool;
  sync.check();
  if (!owned) {
    throw StateError('This agent is no longer available.');
  }
  final head = preflight[1] as NostrEvent?;
  (head == null ? const MeGroups() : sync.decode(head))
      .place(channelId, group: project)
      .encode(sync.community);
  return intent.complete<Channel>(
    sync,
    findChannel: () async => preflight[2] as Channel?,
    createChannel: () => actions.createChannel(
      channelId: channelId,
      name: name,
      channelType: 'stream',
      visibility: 'private',
      labels: const ['buzz.solo'],
      waitForChannelList: false,
    ),
    hasAgent: () async {
      ref.invalidate(channelMembersProvider(channelId));
      final members = await ref.read(channelMembersProvider(channelId).future);
      return members.any((m) => m.pubkey == assistant);
    },
    addAgent: () =>
        actions.addMembers(channelId: channelId, pubkeys: [assistant]),
    isPlaced: () async {
      final latest = await sync.head();
      if (latest == null) return false;
      final groups = sync.decode(latest);
      return groups.channels.contains(channelId) &&
          (project == null || groups.assignments[channelId] == project);
    },
    place: () =>
        sync.update((current) => current.place(channelId, group: project)),
  );
}

List<Map<String, String>> _nativeMeAssistants(
  List<AgentDirectoryEntry> agents,
) => [
  for (final agent in agents)
    {
      'id': agent.pubkey,
      'name': agent.displayName ?? 'Agent',
      'artworkScale': usesInsetAgentArtwork(agent.avatarUrl)
          ? '$animatedAvatarArtworkScale'
          : '1',
      'avatarUrl': agent.avatarUrl ?? '',
    },
];

String? _availableBestie(WidgetRef ref, List<AgentDirectoryEntry> choices) {
  final bestie = ref.read(defaultAgentProvider).value;
  return choices.any((agent) => agent.pubkey == bestie) ? bestie : null;
}
