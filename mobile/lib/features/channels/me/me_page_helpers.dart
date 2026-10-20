part of 'me_page.dart';

class _MeRetry extends StatelessWidget {
  final VoidCallback onRetry;
  const _MeRetry({required this.onRetry});
  @override
  Widget build(BuildContext context) => Center(
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        const Text('Couldn’t load your chats and projects.'),
        TextButton(onPressed: onRetry, child: const Text('Try again')),
      ],
    ),
  );
}

Future<String?> _name(
  BuildContext context,
  String title, {
  String initialValue = '',
}) async {
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final theme = utilitySurfaceThemeData(Theme.of(context));
    try {
      return await const MethodChannel(
        'buzz/profile_text_editor',
      ).invokeMethod<String>('presentProject', {
        'title': title,
        'initialValue': initialValue,
        'placeholder': 'Project name',
        'multiline': false,
        'brightness': theme.brightness.name,
        'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
        'containerBackgroundArgb': theme.colorScheme.surfaceContainerHighest
            .toARGB32(),
        'containerCornerRadius': Radii.container,
      });
    } on MissingPluginException {
      // Use the shared sheet when the native editor is unavailable.
    }
  }
  if (!context.mounted) return null;
  return showBuzzModalBottomSheet<String>(
    context: context,
    title: title,
    isScrollControlled: true,
    requestFocus: true,
    builder: (_) => _NewMeProject(initialValue: initialValue),
  );
}

class _NewMeProject extends HookConsumerWidget {
  final String initialValue;
  const _NewMeProject({this.initialValue = ''});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController(text: initialValue);
    useListenable(controller);
    final name = controller.text.trim();
    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.md,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom + Grid.gutter,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          TextField(
            controller: controller,
            autofocus: true,
            maxLength: 120,
            decoration: const InputDecoration(labelText: 'Project name'),
          ),
          const SizedBox(height: Grid.md),
          FilledButton(
            onPressed: name.isEmpty ? null : () => Navigator.pop(context, name),
            child: const Text('Create project'),
          ),
        ],
      ),
    );
  }
}

class _MeChatRow extends StatelessWidget {
  const _MeChatRow({
    required this.name,
    required this.onTap,
    required this.isUnread,
  });
  final String name;
  final bool isUnread;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final color = navigationPrimaryForeground(
      context,
    ).withValues(alpha: isUnread ? 1 : 0.8);
    return Semantics(
      label: isUnread ? '$name, unread' : name,
      onTap: onTap,
      excludeSemantics: true,
      button: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.xxs,
            vertical: Grid.twelve,
          ),
          child: Row(
            children: [
              SizedBox(
                width: 22,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Icon(BuzzIcons.messageCircle, size: 18, color: color),
                ),
              ),
              const SizedBox(width: Grid.xxs),
              Expanded(
                child: Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: contentListTitleTextStyle.copyWith(
                    color: color,
                    fontWeight: isUnread ? FontWeight.w700 : FontWeight.w400,
                    fontSize: 17,
                    height: 22 / 17,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

Future<void> _offerMeRecovery(BuildContext context, MeSync sync) async {
  final pending = sync.prefs?.getString(sync.pendingKey);
  if (pending == null) return;
  final discard = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Conflicting Solo changes'),
      content: const Text(
        'Desktop and this phone changed the same project or chat. Keep the pending phone change to retry later, or discard it and use the current desktop organization. Your chat messages will be kept.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Keep pending'),
        ),
        TextButton(
          onPressed: () => Navigator.pop(context, true),
          child: const Text('Discard phone change'),
        ),
      ],
    ),
  );
  if (discard == true) await sync.discardPending(pending);
}

// Reconnects replace the transport service while an editor may still be open.
// Retain the reviewed account/community boundary but use its current connection.
MeSync _currentMeSync(WidgetRef ref, MeSync reviewed) {
  final current = ref.read(meSyncProvider);
  if (current.community != reviewed.community ||
      current.pubkey != reviewed.pubkey) {
    throw StateError('Your account or community changed. Open Solo again.');
  }
  return current;
}
