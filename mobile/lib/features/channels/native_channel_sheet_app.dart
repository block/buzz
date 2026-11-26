part of 'channel_actions_sheet.dart';

/// Presentation-only Flutter content inside UIKit. Account state and actions
/// remain in the main isolate; this engine receives a live display model only.
@pragma('vm:entry-point')
Future<void> channelSheetMain() async {
  WidgetsFlutterBinding.ensureInitialized();
  const bridge = MethodChannel('buzz/channel_sheet_content');
  final initial =
      await bridge.invokeMapMethod<String, dynamic>('initial') ?? {};
  runApp(ProviderScope(child: _ChannelSheetApp(initial: initial)));
}

class _ChannelSheetApp extends HookWidget {
  const _ChannelSheetApp({required this.initial});
  final Map<String, dynamic> initial;

  @override
  Widget build(BuildContext context) {
    const bridge = MethodChannel('buzz/channel_sheet_content');
    final data = useState(initial);
    useEffect(() {
      bridge.setMethodCallHandler((call) async {
        if (call.method == 'update') {
          data.value = Map<String, dynamic>.from(call.arguments as Map);
        }
      });
      WidgetsBinding.instance.addPostFrameCallback((_) {
        unawaited(bridge.invokeMethod<void>('ready'));
      });
      return () => bridge.setMethodCallHandler(null);
    }, const []);
    final dark = data.value['dark'] == true;
    final base = dark ? AppTheme.dark() : AppTheme.light();
    final colors = data.value['colors'] as Map? ?? {};
    Color color(String key, Color fallback) =>
        colors[key] is int ? Color(colors[key] as int) : fallback;
    final scheme = base.colorScheme;
    final theme = utilitySheetThemeData(
      base.copyWith(
        colorScheme: scheme.copyWith(
          surface: color('surface', scheme.surface),
          surfaceContainerHighest: color(
            'container',
            scheme.surfaceContainerHighest,
          ),
          surfaceContainer: color('control', scheme.surfaceContainer),
          surfaceContainerHigh: color('high', scheme.surfaceContainerHigh),
          onSurface: color('onSurface', scheme.onSurface),
          onSurfaceVariant: color('onSurfaceVariant', scheme.onSurfaceVariant),
          primary: color('primary', scheme.primary),
          onPrimaryContainer: color(
            'onPrimaryContainer',
            scheme.onPrimaryContainer,
          ),
          error: color('error', scheme.error),
        ),
      ),
    );
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: theme,
      home: ChannelSheetContent(
        data: data.value,
        onSelect: (id) => bridge.invokeMethod<void>('select', id),
      ),
    );
  }
}

/// Uses the same hero, tiles, member rows and cards as the regular details page.
class ChannelSheetContent extends HookWidget {
  const ChannelSheetContent({
    super.key,
    required this.data,
    required this.onSelect,
  });
  final Map<String, dynamic> data;
  final FutureOr<void> Function(String id) onSelect;

  @override
  Widget build(BuildContext context) {
    final solo = data['solo'] == true;
    final nameKey = useMemoized(GlobalKey.new);
    final details = data['details'] is Map
        ? Channel.fromJson(Map<String, dynamic>.from(data['details'] as Map))
        : null;
    final aligned = solo || (details != null && !details.isDm);
    final memberSize = aligned ? Grid.md : Grid.lg;
    final sections = (data['sections'] as List? ?? const [])
        .map(
          (section) => (section as List)
              .map((row) => Map<String, dynamic>.from(row as Map))
              .toList(),
        )
        .toList();
    final title = data['title'] as String? ?? '';
    final avatarUrl = data['heroAvatarUrl'] as String? ?? '';
    VoidCallback? select(Map<String, dynamic> row) =>
        row['enabled'] == true ? () => onSelect(row['id'] as String) : null;
    return Scaffold(
      backgroundColor: context.colors.surface,
      body: SafeArea(
        top: false,
        left: false,
        right: false,
        child: ListView(
          padding: const EdgeInsets.only(bottom: Grid.xs),
          children: [
            if (details != null)
              _ChannelDetailsHero(
                channel: details,
                icon: solo ? BuzzIcons.messageCircle : null,
                nameKey: nameKey,
                displayName: title,
                avatar: avatarUrl.isEmpty
                    ? null
                    : AvatarImage(
                        imageUrl: avatarUrl,
                        radius: 40,
                        isAgent: data['heroIsAgent'] == true,
                        backgroundColor: context.controlSurface,
                        fallback: Text(title.isEmpty ? '?' : title[0]),
                      ),
              ),
            for (var index = 0; index < sections.length; index++)
              if (index == 0 && data['tiles'] == true)
                Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.gutter,
                    vertical: _channelDetailsSectionPadding,
                  ),
                  child: IntrinsicHeight(
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        for (var i = 0; i < sections[index].length; i++) ...[
                          if (i > 0) const SizedBox(width: Grid.xxs),
                          Expanded(
                            child: BuzzActionTile(
                              icon: _sheetIcon(sections[index][i]['symbol']),
                              label: sections[index][i]['label'] as String,
                              iconColor: sections[index][i]['label'] == 'Unmute'
                                  ? context.colors.primary
                                  : null,
                              isEnabled: select(sections[index][i]) != null,
                              onTap: select(sections[index][i]) ?? () {},
                            ),
                          ),
                        ],
                      ],
                    ),
                  ),
                )
              else
                AppListCard(
                  iconColumnWidth: aligned ? Grid.md : null,
                  dividerIndent: !solo && details != null && index == 1
                      ? Grid.xs + memberSize + Grid.xs
                      : null,
                  label:
                      details != null &&
                          index == 1 &&
                          (data['memberLabel'] as String? ?? '').isNotEmpty
                      ? data['memberLabel'] as String
                      : null,
                  verticalPadding: _channelDetailsSectionPadding,
                  children: [
                    for (final row in sections[index])
                      if (row['agent'] case final Map agent)
                        AppListRowRaw(
                          leading: AvatarImage(
                            imageUrl: agent['avatarUrl'] as String?,
                            radius: 16,
                            isAgent: agent['selected'] == true,
                            backgroundColor: context.controlSurface,
                            fallback: agent['selected'] == true
                                ? Text(agent['initial'] as String)
                                : const Icon(BuzzIcons.plus, size: 20),
                          ),
                          title: Text(
                            row['label'] as String,
                            style: context.textTheme.bodyLarge,
                          ),
                          subtitle: Text(
                            row['subtitle'] as String,
                            style: context.textTheme.bodySmall,
                          ),
                          trailing: const _ChannelDetailsChevron(),
                          onTap: select(row),
                        )
                      else if (row['member'] is Map)
                        _ChannelMemberPreviewRow(
                          size: memberSize,
                          padding: aligned ? Grid.xs : Grid.xxs,
                          member: ChannelMember(
                            pubkey: (row['member'] as Map)['pubkey'] as String,
                            role: (row['member'] as Map)['role'] as String,
                            joinedAt: DateTime(1970),
                          ),
                          currentPubkey:
                              (row['member'] as Map)['currentPubkey']
                                  as String?,
                          displayName:
                              (row['member'] as Map)['displayName'] as String?,
                          contextualLabel:
                              ((row['member'] as Map)['contextualLabel']
                                          as String?)
                                      ?.isEmpty ==
                                  true
                              ? null
                              : (row['member'] as Map)['contextualLabel']
                                    as String?,
                          avatarUrl:
                              (row['member'] as Map)['avatarUrl'] as String?,
                          isArchived:
                              (row['member'] as Map)['archived'] == true,
                          onMemberTap: (_, _) => onSelect(row['id'] as String),
                        )
                      else if (row['memberAction'] == true ||
                          details != null &&
                              index == 1 &&
                              (row['label'] == 'Add members' ||
                                  row['label'] == 'See all'))
                        AppListRowRaw(
                          leading: row['label'] == 'See all'
                              ? SizedBox.square(dimension: memberSize)
                              : Container(
                                  width: memberSize,
                                  height: memberSize,
                                  decoration: BoxDecoration(
                                    color: context.colors.surfaceContainer,
                                    shape: BoxShape.circle,
                                  ),
                                  child: Icon(
                                    BuzzIcons.plus,
                                    color: context.colors.onSurfaceVariant,
                                  ),
                                ),
                          title: Text(
                            row['label'] as String,
                            style: context.textTheme.bodyLarge,
                          ),
                          trailing: const _ChannelDetailsChevron(),
                          verticalPadding: aligned ? Grid.xs : Grid.xxs,
                          onTap: select(row),
                        )
                      else
                        AppListRow(
                          icon: _sheetIcon(row['symbol']),
                          title: row['label'] as String,
                          trailing:
                              (row['symbol'] == 'folder' ||
                                  row['symbol'] == 'workspaceFolder')
                              ? const _ChannelDetailsChevron()
                              : row['symbol'] == 'loading'
                              ? BuzzLoadingIndicator(
                                  size: 20,
                                  semanticLabel: row['label'] as String,
                                )
                              : null,
                          titleColor: row['destructive'] == true
                              ? context.colors.error
                              : null,
                          onTap: select(row),
                        ),
                  ],
                ),
          ],
        ),
      ),
    );
  }
}

IconData _sheetIcon(Object? symbol) => switch (symbol) {
  'pin' => BuzzIcons.pinned,
  'pin.slash' => BuzzIcons.pinnedOff,
  'checkmark' => BuzzIcons.checkCheck,
  'envelope' => BuzzIcons.mail,
  'headphones' => BuzzIcons.headphones,
  'workspaceFolder' => BuzzIcons.folder,
  'folder' => BuzzIcons.folderInput,
  'bell' => BuzzIcons.bell,
  'bell.slash' => BuzzIcons.bellOff,
  'gearshape' => BuzzIcons.settings,
  'doc.on.doc' => BuzzIcons.copy,
  'number' => BuzzIcons.hash,
  'rectangle.portrait.and.arrow.right' => BuzzIcons.logOut,
  'archivebox' => BuzzIcons.archive,
  'archiveRestore' => BuzzIcons.archiveRestore,
  'trash' => BuzzIcons.trash2,
  'pencil' => BuzzIcons.pencil,
  'plus' => BuzzIcons.plus,
  'person.2' => BuzzIcons.users,
  'loading' => BuzzIcons.loaderCircle,
  'warning' => BuzzIcons.triangleAlert,
  'join' => BuzzIcons.logIn,
  _ => BuzzIcons.ellipsis,
};
