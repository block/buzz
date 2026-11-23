part of 'channel_actions_sheet.dart';

// A shared declaration for Flutter rows and the UIKit sheet's action model.
class _ChannelSheetRow extends StatelessWidget {
  const _ChannelSheetRow({
    required this.title,
    this.leading,
    this.onTap,
    this.enabled = true,
  });
  final Text title;
  final Widget? leading;
  final FutureOr<void> Function()? onTap;
  final bool enabled;
  @override
  Widget build(BuildContext context) => ListTile(
    title: title,
    leading: leading,
    enabled: enabled,
    onTap: onTap == null ? null : () => onTap!(),
  );
}

const _channelActionsBridge = MethodChannel('buzz/channel_actions_sheet');
int _channelActionsSession = 0;

class _NativeChannelActions extends HookConsumerWidget {
  const _NativeChannelActions({
    required this.groups,
    this.title,
    this.detailsChannel,
    this.heroAvatarUrl,
    this.heroIsAgent = false,
    this.memberLabel,
    this.solo = false,
    this.settings,
  });
  final bool solo;
  final MeChatSettings? settings;
  final Channel? detailsChannel;
  final String? heroAvatarUrl;
  final bool heroIsAgent;
  final String? memberLabel;
  final String? title;
  final List<Widget> groups;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final session = useMemoized(() => '${_channelActionsSession++}');
    final presentationEnded = useRef(false);
    final openingHuddle = useState(false);
    final avatarCancellation = useRef<ValueNotifier<bool>?>(null);
    final resolvedImages = useState(<String, String>{});
    final imageSources = <String, bool>{
      if (settings?.agentAvatarUrl?.isNotEmpty == true)
        settings!.agentAvatarUrl!: true,
      if (heroAvatarUrl?.isNotEmpty == true) heroAvatarUrl!: heroIsAgent,
      for (final group in groups.whereType<SheetActionSection>())
        for (final row in group.children.whereType<_ChannelMemberPreviewRow>())
          if (row.avatarUrl?.isNotEmpty == true)
            row.avatarUrl!: row.member.isBot,
    }..removeWhere((url, _) => url.startsWith('data:'));
    final colors = context.colors;
    useEffect(
      () {
        if (presentationEnded.value) return null;
        var active = true;
        final cancelled = ValueNotifier(false);
        avatarCancellation.value = cancelled;
        if (imageSources.isNotEmpty) {
          final auth = ref.read(mediaGetAuthServiceProvider);
          final client = ref.read(mediaHttpClientProvider);
          final diskCache = ref.read(mediaDiskCacheProvider);
          Future<void> resolveImages() async {
            final images = <String, String>{};
            for (final entry in imageSources.entries) {
              final bytes = await nativeAvatarImage(
                cancelled: cancelled,
                dimension: entry.key == heroAvatarUrl ? 240 : 120,
                url: entry.key,
                initial: '?',
                isAgent: entry.value,
                background: colors.surfaceContainer,
                foreground: colors.onPrimaryContainer,
                networkImages: (url) => smallMediaImageProviders(
                  url: url,
                  auth: auth,
                  client: client,
                  diskCache: diskCache,
                ),
              );
              if (!active || cancelled.value) return;
              if (bytes != null) {
                images[entry.key] =
                    'data:image/png;base64,${base64Encode(bytes)}';
              }
            }
            if (active) resolvedImages.value = images;
          }

          unawaited(resolveImages());
        }
        return () {
          active = false;
          cancelled.value = true;
          cancelled.dispose();
        };
      },
      [
        imageSources.toString(),
        colors.surfaceContainer,
        colors.onPrimaryContainer,
      ],
    );
    String imageSource(String? url) => url?.startsWith('data:') == true
        ? url!
        : resolvedImages.value[url] ?? '';
    final callbacks = <String, FutureOr<void> Function()>{};
    final sections = <List<Map<String, Object>>>[];
    Map<String, Object> entry(
      String label,
      FutureOr<void> Function()? action, {
      bool destructive = false,
      IconData? icon,
      String? id,
    }) {
      if (action != null) callbacks[id ?? label] = action;
      return {
        'id': id ?? label,
        'label': label,
        'enabled': action != null,
        'destructive': destructive,
        'symbol': _channelActionSymbol(icon),
      };
    }

    for (final group in groups) {
      if (group is _ChannelQuickActionsRow) {
        sections.add([
          entry(
            group.isUnread ? 'Mark Read' : 'Mark Unread',
            group.onToggleRead,
            icon: BuzzIcons.checkCheck,
          ),
        ]);
      } else if (group is SheetActionSection) {
        final rows = <Map<String, Object>>[];
        for (final row in group.children) {
          if (row is _ChannelMemberPreviewRow) {
            rows.add({
              ...entry(
                row.member.pubkey,
                () => row.onMemberTap(context, row.member.pubkey),
                id: 'member:${row.member.pubkey}',
              ),
              'member': {
                'pubkey': row.member.pubkey,
                'role': row.member.role,
                'currentPubkey': row.currentPubkey ?? '',
                'displayName': row.displayName ?? '',
                'contextualLabel': row.contextualLabel ?? '',
                'avatarUrl': imageSource(row.avatarUrl),
                'archived': row.isArchived,
              },
            });
          } else if (row is AppListRow) {
            final isWorkspace = row.icon == BuzzIcons.folder;
            rows.add({
              ...entry(
                row.title,
                row.onTap,
                icon: row.icon,
                id: isWorkspace
                    ? 'workspace'
                    : row.icon == BuzzIcons.headphones
                    ? 'huddle'
                    : row.title,
                destructive: row.titleColor == colors.error,
              ),
              if (row.subtitle != null) 'subtitle': row.subtitle!,
              if (row.subtitle != null && settings != null)
                'agent': {
                  'avatarUrl': imageSource(settings!.agentAvatarUrl),
                  'initial': settings!.agentInitial,
                  'selected': settings!.hasAgent,
                },
            });
          } else if (row is AppListRowRaw && row.title is Text) {
            final label = (row.title as Text).data!;
            rows.add({
              ...entry(label, row.onTap, icon: BuzzIcons.plus),
              'memberAction': true,
            });
          } else if (row is _ChannelSheetRow) {
            rows.add(
              entry(
                row.title.data!,
                row.enabled ? row.onTap : null,
                icon: row.leading is Icon ? (row.leading as Icon).icon : null,
              ),
            );
          } else if (row is _ActionTile) {
            rows.add(
              entry(
                row.label,
                row.onTap,
                destructive: row.destructive,
                icon: row.icon,
              ),
            );
          }
        }
        if (rows.isNotEmpty || detailsChannel != null) sections.add(rows);
      }
    }
    final latest = useRef(callbacks)..value = callbacks;
    final payload = {
      'session': session,
      'solo': solo,
      'title': title ?? '',
      if (detailsChannel != null) 'details': detailsChannel!.toJson(),
      'heroAvatarUrl': imageSource(heroAvatarUrl),
      'heroIsAgent': heroIsAgent,
      'memberLabel': memberLabel ?? '',
      'tiles':
          detailsChannel != null ||
          groups.firstOrNull is _ChannelQuickActionsRow,
      'colors': {
        for (final item in <String, Color>{
          'surface': context.colors.surface,
          'container': context.colors.surfaceContainerHighest,
          'control': context.colors.surfaceContainer,
          'high': context.colors.surfaceContainerHigh,
          'onSurface': context.colors.onSurface,
          'onSurfaceVariant': context.colors.onSurfaceVariant,
          'primary': context.colors.primary,
          'onPrimaryContainer': context.colors.onPrimaryContainer,
          'error': context.colors.error,
        }.entries)
          item.key: item.value.toARGB32(),
      },
      'dark': Theme.of(context).brightness == Brightness.dark,
      'sections': sections,
    };
    final latestPayload = useRef(payload)..value = payload;
    useEffect(() {
      var disposed = false;
      WidgetsBinding.instance.addPostFrameCallback((_) async {
        if (disposed) return;
        final route = ModalRoute.of(context);
        try {
          final selected = await _channelActionsBridge.invokeMethod<String>(
            'present',
            latestPayload.value,
          );
          if (disposed || !context.mounted) return;
          presentationEnded.value = true;
          avatarCancellation.value?.value = true;
          if (selected == 'huddle') openingHuddle.value = true;
          final action = latest.value[selected];
          if (action != null) await action();
        } on PlatformException catch (error) {
          if (context.mounted) {
            ScaffoldMessenger.of(context).showFeedbackSnackBar(
              SnackBar(
                content: Text(
                  'Could not open channel actions: ${error.message ?? error.code}',
                ),
              ),
              kind: ToastKind.error,
            );
          }
        } on MissingPluginException {
          if (route is _NativeChannelSheetRoute && route.hasFallback) {
            route.unavailable = true;
          } else if (context.mounted) {
            ScaffoldMessenger.of(context).showFeedbackSnackBar(
              const SnackBar(
                content: Text('Channel actions are unavailable in this build.'),
              ),
              kind: ToastKind.error,
            );
          }
        } finally {
          if (!disposed && context.mounted && route != null && route.isActive) {
            route.navigator!.removeRoute(route);
          }
        }
      });
      return () {
        disposed = true;
        unawaited(
          _channelActionsBridge
              .invokeMethod<void>('dismiss', {'session': session})
              .catchError((Object _) {}),
        );
      };
    }, [session]);
    useEffect(() {
      unawaited(
        _channelActionsBridge
            .invokeMethod<void>('update', payload)
            .catchError((Object _) {}),
      );
      return null;
    }, [payload.toString()]);
    return openingHuddle.value
        ? const Center(
            child: BuzzLoadingIndicator(semanticLabel: 'Opening Huddle'),
          )
        : const SizedBox.shrink();
  }
}

String _channelActionSymbol(IconData? icon) => switch (icon) {
  BuzzIcons.loaderCircle => 'loading',
  BuzzIcons.triangleAlert => 'warning',
  BuzzIcons.logIn => 'join',
  BuzzIcons.pencil => 'pencil',
  BuzzIcons.plus => 'plus',
  BuzzIcons.users => 'person.2',
  BuzzIcons.pinned => 'pin',
  BuzzIcons.pinnedOff => 'pin.slash',
  BuzzIcons.checkCheck => 'checkmark',
  BuzzIcons.mail => 'envelope',
  BuzzIcons.folder => 'workspaceFolder',
  BuzzIcons.headphones => 'headphones',
  BuzzIcons.starOff => 'star',
  BuzzIcons.folderInput => 'folder',
  BuzzIcons.bell => 'bell',
  BuzzIcons.bellOff => 'bell.slash',
  BuzzIcons.settings => 'gearshape',
  BuzzIcons.copy => 'doc.on.doc',
  BuzzIcons.hash => 'number',
  BuzzIcons.logOut => 'rectangle.portrait.and.arrow.right',
  BuzzIcons.archive => 'archivebox',
  BuzzIcons.archiveRestore => 'archiveRestore',
  BuzzIcons.trash2 => 'trash',
  _ => 'ellipsis',
};
