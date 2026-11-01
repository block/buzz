part of 'me_page.dart';

/// A conversation draft: no channel is created until the first send.
class _MeChatDraft extends HookConsumerWidget {
  const _MeChatDraft();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final originalScope = useMemoized(() => (sync.community, sync.pubkey));
    final pending = useState(MeChatCreation.pending(sync));
    final channelId = useMemoized(() => pending.value?.id ?? const Uuid().v4());
    final manualAgent = useState<String?>(null);
    final project = useState<String?>(pending.value?.project);
    final created = useState<Channel?>(null);
    final busy = useState(false);
    final composerBusy = useState(false);
    final error = useState<String?>(null);
    final directory = ref.watch(meAssistantsProvider);
    final bestie = ref.watch(defaultAgentProvider).value;
    final choices = directory.value ?? [];
    final selected =
        pending.value?.agent ??
        manualAgent.value ??
        (choices.any((agent) => agent.pubkey == bestie) ? bestie : null);
    final profile = ref.watch(userCacheProvider)[selected];
    final agent = choices
        .where((agent) => agent.pubkey == selected)
        .firstOrNull;
    final name =
        agent?.displayName ??
        profile?.label ??
        (selected == null ? 'Choose an agent' : 'Agent');
    final avatarInitial = name.isEmpty ? 'A' : name.characters.first;
    final scopeChanged = originalScope != (sync.community, sync.pubkey);
    final frozen =
        pending.value != null ||
        busy.value ||
        composerBusy.value ||
        scopeChanged;
    final workspace = useMeWorkspace(
      context,
      ref,
      enabled: !scopeChanged,
      selected: project.value,
      locked: frozen,
      onSelected: (value) {
        if (!busy.value && pending.value == null && !sync.disposed) {
          project.value = value;
        }
      },
    );
    if (created.value case final channel?) {
      return ChannelDetailPage(
        channel: channel,
        personalChat: true,
        initialAssistant: selected,
      );
    }

    Future<void> choose() async {
      final picked = await showBuzzModalBottomSheet<String>(
        context: context,
        title: 'Agent',
        isScrollControlled: true,
        showDragHandle: true,
        builder: (_) => MeAssistantPickerSheet(selected: selected),
      );
      if (context.mounted && picked != null && !sync.disposed) {
        manualAgent.value = picked;
      }
    }

    Future<void> send(
      String content,
      List<String> mentions, {
      List<List<String>> mediaTags = const [],
    }) async {
      if (workspace.busy) {
        throw Exception('Wait for the workspace to finish saving.');
      }
      if (busy.value) throw Exception('Wait for this chat to finish sending.');
      if (scopeChanged || sync.disposed) {
        throw Exception('Your account or community changed. Return to Solo.');
      }
      final saved = pending.value;
      final recipient = saved?.agent ?? selected;
      if (recipient == null) {
        throw Exception('Choose an agent in the chat header before sending.');
      }
      if (saved == null && !choices.any((agent) => agent.pubkey == recipient)) {
        throw Exception('Choose an available agent in the chat header.');
      }
      if (utf8.encode(content).length > 8192 ||
          utf8.encode(jsonEncode(mediaTags)).length > 16384) {
        throw Exception(
          'Message is too long. Keep your first message under 8 KB.',
        );
      }
      busy.value = true;
      error.value = null;
      try {
        final result = await _createMeChat(
          ref,
          sync,
          channelId,
          saved?.name ?? startingMeChatTitle(content),
          recipient,
          saved?.project ?? project.value,
          initialContent: saved?.initialMessage?.content ?? content,
          initialMediaTags: mediaTags,
        );
        if (context.mounted) {
          manualAgent.value = recipient;
          created.value = result;
        }
      } catch (failure) {
        if (context.mounted) {
          pending.value = MeChatCreation.pending(sync);
          error.value = failure.toString().replaceFirst('Bad state: ', '');
        }
        // ComposeBar treats StateError as a community switch. Preserve the
        // actual failure and its draft through its ordinary error path.
        throw Exception(failure.toString().replaceFirst('Bad state: ', ''));
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    final label = selected == null ? 'Choose an agent' : 'Change agent: $name';
    return PopScope(
      canPop: !busy.value && !composerBusy.value && !workspace.busy,
      child: FrostedScaffold(
        appBar: FrostedAppBar(
          leading: BackButton(
            onPressed: () => Navigator.of(context).maybePop(),
          ),
          nativeLeading: IosNavigationAction(
            label: 'Back',
            symbol: 'chevron.backward',
            onPressed: () => Navigator.of(context).maybePop(),
          ),
          title: const Text('New chat'),
          nativeTitle: 'New chat',
          centerTitle: true,
          nativeActions: [
            workspace.nativeAction,
            IosNavigationAction(
              label: label,
              symbol: 'person.crop.circle',
              sharesBackground: false,
              avatarIdentity: selected,
              avatarIsAgent: selected != null,
              imageUrl: agent?.avatarUrl ?? profile?.avatarUrl,
              avatarInitial: avatarInitial,
              onPressed: frozen || workspace.busy ? null : choose,
            ),
          ],
          actions: [
            workspace.action,
            IconButton(
              key: const ValueKey('new-chat-agent'),
              tooltip: label,
              onPressed: frozen || workspace.busy ? null : choose,
              icon: AvatarImage(
                imageUrl: agent?.avatarUrl ?? profile?.avatarUrl,
                radius: 16,
                isAgent: selected != null,
                backgroundColor: context.controlSurface,
                fallback: selected == null
                    ? const Icon(BuzzIcons.plus, size: 20)
                    : Text(avatarInitial),
              ),
            ),
          ],
        ),
        body: Padding(
          padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
          child: Column(
            children: [
              if (directory.isLoading) const LinearProgressIndicator(),
              Expanded(
                child: Center(
                  child: Padding(
                    padding: const EdgeInsets.all(Grid.gutter),
                    child: Text(
                      scopeChanged
                          ? 'Your account or community changed. Return to Solo to start a chat.'
                          : pending.value?.initialMessage?.content ??
                                'What would you like to work on?',
                      textAlign: TextAlign.center,
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ),
                ),
              ),
              if (error.value != null)
                Padding(
                  padding: const EdgeInsets.all(Grid.gutter),
                  child: Semantics(liveRegion: true, child: Text(error.value!)),
                ),
              if (!scopeChanged && pending.value != null)
                SafeArea(
                  child: Padding(
                    padding: const EdgeInsets.all(Grid.gutter),
                    child: FilledButton(
                      onPressed: busy.value
                          ? null
                          : () async {
                              try {
                                await send(
                                  pending.value!.initialMessage!.content,
                                  const [],
                                );
                              } catch (_) {
                                /* The durable error is shown above. */
                              }
                            },
                      child: Text(busy.value ? 'Sending…' : 'Retry'),
                    ),
                  ),
                ),
              if (!scopeChanged && pending.value == null)
                ComposeBar(
                  channelId: 'new-solo',
                  interactionLocked: workspace.busy,
                  hintText: 'Message $name',
                  draftMembers: [
                    ChannelMember(
                      pubkey: sync.pubkey,
                      role: 'owner',
                      joinedAt: DateTime(2026),
                    ),
                    if (selected != null)
                      ChannelMember(
                        pubkey: selected,
                        role: 'bot',
                        joinedAt: DateTime(2026),
                        displayName: name,
                      ),
                  ],
                  onSendingChanged: (value) => composerBusy.value = value,
                  onSend: send,
                ),
            ],
          ),
        ),
      ),
    );
  }
}
