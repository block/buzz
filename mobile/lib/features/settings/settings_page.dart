import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';

import '../../shared/auth/auth.dart';
import '../../shared/clipboard_utils.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../profile/set_status_sheet.dart';
import '../profile/user_status_provider.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import '../custom_emoji/custom_emoji_render.dart';
import 'theme_picker_page.dart';

class SettingsPage extends HookConsumerWidget {
  const SettingsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final config = ref.watch(relayConfigProvider);
    final selectedAccent = ref.watch(accentProvider);
    final selectedScheme = ref.watch(schemeProvider);
    final packageInfoFuture = useMemoized(() => PackageInfo.fromPlatform());
    final packageInfo = useFuture(packageInfoFuture);

    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Settings')),
      body: Column(
        children: [
          Expanded(
            child: ListView(
              padding: EdgeInsets.only(
                top: frostedAppBarHeight(context),
                bottom: Grid.xs,
              ),
              children: [
                const SizedBox(height: Grid.xxs),

                // Status — flush header row, like Slack's profile/status block.
                _StatusRow(),

                // Appearance
                AppListSection(
                  label: 'Appearance',
                  children: [
                    AppListRow(
                      icon: LucideIcons.palette,
                      title: 'Color Scheme',
                      subtitle: selectedScheme == null
                          ? 'Default ($defaultSchemeDisplayName)'
                          : findTheme(selectedScheme)?.displayName ??
                                selectedScheme,
                      trailing: Icon(
                        LucideIcons.chevronRight,
                        size: 18,
                        color: context.colors.onSurfaceVariant,
                      ),
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) => const ThemePickerPage(),
                        ),
                      ),
                    ),
                    Padding(
                      padding: const EdgeInsets.fromLTRB(
                        Grid.gutter,
                        Grid.xxs,
                        Grid.gutter,
                        Grid.twelve,
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            'Accent Color',
                            style: context.textTheme.bodyMedium?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                          const SizedBox(height: Grid.twelve),
                          Wrap(
                            spacing: Grid.xxs,
                            runSpacing: Grid.xxs,
                            children: [
                              for (var i = 0; i < accentColors.length; i++)
                                _AccentSwatch(
                                  color:
                                      context.colors.brightness ==
                                          Brightness.light
                                      ? accentColors[i].light
                                      : accentColors[i].dark,
                                  label: accentColors[i].name,
                                  selected: selectedAccent == i,
                                  onTap: () => ref
                                      .read(accentProvider.notifier)
                                      .setAccent(i),
                                ),
                            ],
                          ),
                        ],
                      ),
                    ),
                  ],
                ),

                // Connection
                AppListSection(
                  label: 'Connection',
                  children: [
                    AppListRow(
                      icon: LucideIcons.server,
                      title: 'Connected to',
                      subtitle: config.baseUrl,
                    ),
                    if (config.nsec != null && config.nsec!.isNotEmpty)
                      Builder(
                        builder: (context) {
                          final privHex = nostr.Nip19.decode(
                            payload: config.nsec!,
                          ).data;
                          final pubkey = privHex.isNotEmpty
                              ? nostr.Keys(privHex).public
                              : 'unknown';
                          return AppListRow(
                            icon: LucideIcons.key,
                            title: 'Identity (pubkey)',
                            subtitle: pubkey,
                            subtitleStyle: context.textTheme.bodySmall
                                ?.copyWith(
                                  color: context.colors.onSurfaceVariant,
                                  fontFamily: 'GeistMono',
                                  fontSize: 11,
                                ),
                            subtitleMaxLines: 2,
                            trailing: IconButton(
                              icon: const Icon(LucideIcons.copy, size: 16),
                              onPressed: () async {
                                await copyToClipboard(
                                  context,
                                  pubkey,
                                  message: 'Pubkey copied',
                                );
                              },
                            ),
                          );
                        },
                      ),
                    Padding(
                      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
                      child: Center(
                        child: TextButton.icon(
                          onPressed: () => _confirmSignOut(context, ref),
                          icon: const Icon(LucideIcons.logOut, size: 18),
                          label: const Text('Remove Workspace'),
                          style: TextButton.styleFrom(
                            foregroundColor: context.colors.error,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
          if (packageInfo.hasData)
            SafeArea(
              top: false,
              child: Padding(
                padding: const EdgeInsets.only(bottom: Grid.xs, top: Grid.xxs),
                child: Center(
                  child: Text(
                    'v${packageInfo.data!.version}',
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant.withValues(
                        alpha: 0.6,
                      ),
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }

  void _confirmSignOut(BuildContext context, WidgetRef ref) {
    showDialog<void>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Remove Workspace'),
        content: const Text(
          'This will disconnect this workspace. You will need '
          'to scan a new pairing code to reconnect.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () {
              Navigator.of(ctx).pop(); // close dialog
              // Pop all pushed routes back to root so MaterialApp.home
              // rebuilds to PairingPage when auth state changes.
              Navigator.of(context).popUntil((route) => route.isFirst);
              ref.read(authProvider.notifier).signOut();
            },
            style: FilledButton.styleFrom(
              backgroundColor: Theme.of(ctx).colorScheme.error,
            ),
            child: const Text('Remove'),
          ),
        ],
      ),
    );
  }
}

class _StatusRow extends ConsumerWidget {
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final statusAsync = ref.watch(userStatusProvider);
    final status = statusAsync.asData?.value;
    final hasStatus = status != null && !status.isEmpty;

    return AppListRowRaw(
      leading: _StatusEmojiIcon(emoji: status?.emoji ?? ''),
      title: Text(
        hasStatus
            ? (status.text.isNotEmpty ? status.text : status.emoji)
            : 'Set a status',
        style: hasStatus
            ? context.textTheme.bodyLarge
            : context.textTheme.bodyLarge?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
      ),
      subtitle: hasStatus
          ? Text(
              'Tap to update',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            )
          : null,
      onTap: () => showSetStatusSheet(context, currentStatus: status),
    );
  }
}

class _StatusEmojiIcon extends ConsumerWidget {
  final String emoji;

  const _StatusEmojiIcon({required this.emoji});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (emoji.isEmpty) {
      return const Text('\u{1F4AC}', style: TextStyle(fontSize: 20));
    }
    final shortcode = emoji.startsWith(':') && emoji.endsWith(':')
        ? emoji.substring(1, emoji.length - 1).toLowerCase()
        : null;
    if (shortcode != null) {
      for (final entry in ref.watch(customEmojiListProvider)) {
        if (entry.shortcode == shortcode) {
          return CustomEmojiImage(
            shortcode: shortcode,
            url: entry.url,
            size: 24,
          );
        }
      }
    }
    return Text(emoji, style: const TextStyle(fontSize: 20));
  }
}

class _AccentSwatch extends StatelessWidget {
  const _AccentSwatch({
    required this.color,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final Color color;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: label,
      child: GestureDetector(
        onTap: onTap,
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 150),
          width: 36,
          height: 36,
          decoration: BoxDecoration(
            color: color,
            borderRadius: BorderRadius.circular(Radii.md),
            border: selected
                ? Border.all(color: context.colors.onSurface, width: 2.5)
                : Border.all(color: color.withValues(alpha: 0.4), width: 1),
          ),
          child: selected
              ? Icon(
                  LucideIcons.check,
                  size: 16,
                  color: contrastForeground(color),
                )
              : null,
        ),
      ),
    );
  }
}
