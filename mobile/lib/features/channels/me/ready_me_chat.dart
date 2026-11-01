part of 'me_page.dart';

/// New Solo conversations start at the composer, with no setup form.
class _ReadyMeChat extends HookConsumerWidget {
  const _ReadyMeChat();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final originalSync = useMemoized(() => sync);
    final id = useMemoized(() => const Uuid().v4());
    final defaultAgent = ref.watch(defaultAgentProvider);
    final selected = useState<String?>(null);
    final agent = selected.value ?? defaultAgent.value;
    final workspace = useState<String?>(null);
    final groups = ref.watch(meGroupsProvider);
    final directory = ref.watch(meAssistantsProvider);
    final busy = useState(false);
    final frozenAgent = useRef<String?>(null);
    final frozenName = useRef<String?>(null);
    final frozenWorkspace = useRef<String?>(null);
    final created = useRef<Channel?>(null);
    final focus = useFocusNode();
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) focus.requestFocus();
      });
      return null;
    }, const []);
    final choices = directory.value ?? const <AgentDirectoryEntry>[];
    final label =
        choices.where((a) => a.pubkey == agent).firstOrNull?.displayName ??
        ref.watch(userCacheProvider)[agent]?.label ??
        'Bestie';
    final locked = busy.value || frozenAgent.value != null;
    Future<void> chooseAgent() async {
      final picked = await showBuzzModalBottomSheet<String>(
        context: context,
        title: 'Agent',
        isScrollControlled: true,
        builder: (_) => MeAssistantList(choices: choices, selected: agent),
      );
      if (context.mounted && picked != null) selected.value = picked;
    }

    final workspaces = groups.value?.groups ?? const <Map<String, dynamic>>[];
    final workspaceLabel =
        workspaces.where((g) => g['id'] == workspace.value).firstOrNull?['name']
            as String? ??
        'Chats';
    return FrostedScaffold(
      appBar: FrostedAppBar(
        title: const Text('New chat'),
        nativeTitle: 'New chat',
        nativeActions: [
          IosNavigationAction(
            label: 'Workspace: $workspaceLabel',
            icon: BuzzIcons.folder,
            sharesBackground: false,
            children: locked
                ? const []
                : [
                    IosNavigationAction(
                      label: 'Chats',
                      selected: workspace.value == null,
                      onPressed: () => workspace.value = null,
                    ),
                    for (final group in workspaces)
                      IosNavigationAction(
                        label: group['name'] as String,
                        selected: workspace.value == group['id'],
                        onPressed: () =>
                            workspace.value = group['id'] as String,
                      ),
                  ],
          ),
          IosNavigationAction(
            label: 'Agent: $label',
            symbol: 'person.crop.circle',
            avatarIdentity: agent,
            avatarIsAgent: agent != null,
            imageUrl: ref.watch(userCacheProvider)[agent]?.avatarUrl,
            avatarInitial: label.isEmpty
                ? 'A'
                : label.characters.first.toUpperCase(),
            sharesBackground: false,
            onPressed: locked ? null : chooseAgent,
          ),
        ],
        actions: [
          PopupMenuButton<String>(
            key: const ValueKey('new-session-project'),
            tooltip: 'Workspace: $workspaceLabel',
            icon: const Icon(BuzzIcons.folder),
            enabled: !locked,
            onSelected: (v) => workspace.value = v.isEmpty ? null : v,
            itemBuilder: (_) => [
              CheckedPopupMenuItem(
                value: '',
                checked: workspace.value == null,
                child: const Text('Chats'),
              ),
              for (final group in workspaces)
                CheckedPopupMenuItem(
                  checked: workspace.value == group['id'],
                  value: group['id'] as String,
                  child: Text(group['name'] as String),
                ),
            ],
          ),
          IconButton(
            key: const ValueKey('new-session-assistant'),
            tooltip: 'Agent: $label',
            icon: const Icon(BuzzIcons.bot),
            onPressed: locked ? null : chooseAgent,
          ),
        ],
      ),
      body: Column(
        children: [
          const Expanded(child: SizedBox.expand()),
          if (busy.value) const LinearProgressIndicator(),
          ComposeBar(
            channelId: id,
            focusNode: focus,
            hintText: 'Message $label',
            onSend:
                (
                  content,
                  mentions, {
                  mediaTags = const <List<String>>[],
                }) async {
                  if (busy.value) {
                    throw StateError('A message is already being sent.');
                  }
                  if (sync != originalSync) {
                    throw StateError('Your account or community changed.');
                  }
                  sync.check();
                  if (agent == null) {
                    throw StateError('Choose an agent in the chat header.');
                  }
                  busy.value = true;
                  try {
                    frozenAgent.value ??= agent;
                    frozenName.value ??= content.trim().isEmpty
                        ? 'New chat'
                        : String.fromCharCodes(content.trim().runes.take(80));
                    if (created.value == null) {
                      // Freeze the creation intent across retries, including no workspace.
                      if (MeChatCreation.pending(sync) == null) {
                        frozenWorkspace.value = workspace.value;
                      }
                      created.value = await _createMeChat(
                        ref,
                        sync,
                        id,
                        frozenName.value!,
                        frozenAgent.value!,
                        frozenWorkspace.value,
                      );
                    }
                    if (!context.mounted) return;
                    final recipients = await meRecipients(
                      ref,
                      id,
                      frozenAgent.value,
                      mentions,
                    );
                    await ref.read(meSendMessageProvider)(
                      channelId: id,
                      content: content,
                      mentionPubkeys: recipients,
                      channel: created.value,
                      mediaTags: mediaTags,
                    );
                    if (context.mounted) {
                      Navigator.of(
                        context,
                      ).pop((created.value!, frozenAgent.value!));
                    }
                  } finally {
                    if (context.mounted) busy.value = false;
                  }
                },
          ),
        ],
      ),
    );
  }
}
