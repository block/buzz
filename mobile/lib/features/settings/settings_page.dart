import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';

import '../../shared/auth/auth.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../profile/set_status_sheet.dart';
import '../profile/user_status_provider.dart';
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
      body: ListView(
        padding: EdgeInsets.only(
          top: frostedAppBarHeight(context),
          left: Grid.xs,
          right: Grid.xs,
          bottom: Grid.xs,
        ),
        children: [
          // Connection info
          Text('Connection', style: context.textTheme.titleMedium),
          const SizedBox(height: Grid.twelve),
          ListTile(
            leading: const Icon(LucideIcons.server),
            title: const Text('Connected to'),
            subtitle: Text(
              config.baseUrl,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(Radii.md),
              side: BorderSide(color: context.colors.outlineVariant),
            ),
          ),
          if (config.nsec != null && config.nsec!.isNotEmpty) ...[
            const SizedBox(height: Grid.xxs),
            Builder(
              builder: (context) {
                final privHex = nostr.Nip19.decode(payload: config.nsec!).data;
                final pubkey = privHex.isNotEmpty
                    ? nostr.Keys(privHex).public
                    : 'unknown';
                return ListTile(
                  leading: const Icon(LucideIcons.key),
                  title: const Text('Identity (pubkey)'),
                  subtitle: Text(
                    pubkey,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                      fontFamily: 'GeistMono',
                      fontSize: 11,
                    ),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                  ),
                  trailing: IconButton(
                    icon: const Icon(LucideIcons.copy, size: 16),
                    onPressed: () {
                      Clipboard.setData(ClipboardData(text: pubkey));
                      ScaffoldMessenger.of(context).showSnackBar(
                        const SnackBar(
                          content: Text('Pubkey copied'),
                          duration: Duration(seconds: 2),
                        ),
                      );
                    },
                  ),
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(Radii.md),
                    side: BorderSide(color: context.colors.outlineVariant),
                  ),
                );
              },
            ),
          ],
          const SizedBox(height: Grid.twelve),
          OutlinedButton.icon(
            onPressed: () => _confirmSignOut(context, ref),
            icon: const Icon(LucideIcons.logOut),
            label: const Text('Remove Workspace'),
            style: OutlinedButton.styleFrom(
              foregroundColor: context.colors.error,
            ),
          ),

          const SizedBox(height: Grid.sm),

          // Status
          _StatusSection(),

          const SizedBox(height: Grid.sm),

          // Appearance
          Text('Appearance', style: context.textTheme.titleMedium),
          const SizedBox(height: Grid.twelve),

          // Color scheme picker — navigates to dedicated page
          ListTile(
            leading: const Icon(LucideIcons.palette),
            title: const Text('Color Scheme'),
            subtitle: Text(
              selectedScheme == null
                  ? 'Default (Catppuccin)'
                  : findTheme(selectedScheme)?.displayName ?? selectedScheme,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            trailing: const Icon(LucideIcons.chevronRight, size: 18),
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const ThemePickerPage()),
            ),
          ),

          const SizedBox(height: Grid.xs),

          // Accent color picker
          Text('Accent Color', style: context.textTheme.titleSmall),
          const SizedBox(height: Grid.xxs),
          Wrap(
            spacing: Grid.xxs,
            runSpacing: Grid.xxs,
            children: [
              // Default (Mauve) swatch
              _AccentSwatch(
                color: context.colors.brightness == Brightness.light
                    ? const Color(0xFF8839EF)
                    : const Color(0xFFC6A0F6),
                label: 'Mauve',
                selected: selectedAccent == defaultAccentIndex,
                onTap: () => ref
                    .read(accentProvider.notifier)
                    .setAccent(defaultAccentIndex),
              ),
              for (var i = 0; i < accentColors.length; i++)
                _AccentSwatch(
                  color: context.colors.brightness == Brightness.light
                      ? accentColors[i].light
                      : accentColors[i].dark,
                  label: accentColors[i].name,
                  selected: selectedAccent == i,
                  onTap: () => ref.read(accentProvider.notifier).setAccent(i),
                ),
            ],
          ),
          if (packageInfo.hasData) ...[
            const SizedBox(height: Grid.sm),
            Center(
              child: Text(
                'v${packageInfo.data!.version}',
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant.withValues(alpha: 0.6),
                ),
              ),
            ),
          ],
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

class _StatusSection extends ConsumerWidget {
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final statusAsync = ref.watch(userStatusProvider);
    final status = statusAsync.asData?.value;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Status', style: context.textTheme.titleMedium),
        const SizedBox(height: Grid.twelve),
        ListTile(
          leading: Text(
            status != null && status.emoji.isNotEmpty
                ? status.emoji
                : '\u{1F4AC}',
            style: const TextStyle(fontSize: 20),
          ),
          title: Text(
            status != null && !status.isEmpty
                ? status.text.isNotEmpty
                      ? status.text
                      : status.emoji
                : 'Set a status',
            style: status != null && !status.isEmpty
                ? null
                : context.textTheme.bodyMedium?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
          ),
          subtitle: status != null && !status.isEmpty
              ? Text(
                  'Tap to update',
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                )
              : null,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.md),
            side: BorderSide(color: context.colors.outlineVariant),
          ),
          onTap: () => showSetStatusSheet(context, currentStatus: status),
        ),
      ],
    );
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
