import 'package:app_badge_plus/app_badge_plus.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'features/channels/unread_badge/unread_badge_provider.dart';
import 'features/home/home_page.dart';
import 'features/pairing/pairing_page.dart';
import 'features/channels/agent_activity/observer_subscription.dart';
import 'features/profile/user_status_cache_provider.dart';
import 'shared/auth/auth.dart';
import 'shared/relay/relay.dart';
import 'shared/theme/theme.dart';

class App extends HookConsumerWidget {
  const App({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final themeMode = ref.watch(themeProvider);
    final accentIndex = ref.watch(accentProvider);
    final schemeName = ref.watch(schemeProvider);
    final authState = ref.watch(authProvider);

    final resolved = resolveSchemes(schemeName);
    final lightScheme = applyAccent(resolved.light, accentIndex);
    final darkScheme = applyAccent(resolved.dark, accentIndex);
    // Default and named schemes can force light or dark mode; otherwise
    // respect the user's ThemeMode preference.
    final effectiveMode = resolved.forcedMode ?? themeMode;

    // Eagerly initialize websocket session and lifecycle observer when
    // authenticated. These providers connect and manage the websocket.
    if (authState.value?.status == AuthStatus.authenticated) {
      ref.watch(relaySessionProvider);
      ref.watch(observerRelayProvider);
      ref.watch(appLifecycleProvider);
      ref.watch(userStatusCacheProvider);
    }

    void applyBadge(UnreadBadgeState state) {
      if (state.highPriorityCount > 0) {
        AppBadgePlus.updateBadge(state.highPriorityCount);
      } else if (state.generalUnreadCount > 0) {
        AppBadgePlus.updateBadge(1);
      } else {
        AppBadgePlus.updateBadge(0);
      }
    }

    useEffect(() {
      applyBadge(ref.read(unreadBadgeProvider));
      return null;
    }, const []);
    ref.listen<UnreadBadgeState>(unreadBadgeProvider, (_, next) {
      applyBadge(next);
    });

    return MaterialApp(
      title: 'Buzz',
      theme: AppTheme.light(colorScheme: lightScheme),
      darkTheme: AppTheme.dark(colorScheme: darkScheme),
      themeMode: effectiveMode,
      home: authState.when(
        loading: () => const _SplashScreen(),
        error: (_, _) => const PairingPage(),
        data: (state) => switch (state.status) {
          AuthStatus.authenticated => const HomePage(),
          AuthStatus.offline => const _OfflineScreen(),
          _ => const PairingPage(),
        },
      ),
    );
  }
}

class _SplashScreen extends StatelessWidget {
  const _SplashScreen();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(body: Center(child: CircularProgressIndicator()));
  }
}

class _OfflineScreen extends ConsumerWidget {
  const _OfflineScreen();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Scaffold(
      body: Center(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                LucideIcons.wifiOff,
                size: 48,
                color: context.colors.onSurfaceVariant,
              ),
              const SizedBox(height: Grid.xs),
              Text(
                'Unable to reach relay',
                style: context.textTheme.titleMedium,
              ),
              const SizedBox(height: Grid.xxs),
              Text(
                'Your pairing is saved — check your connection and try again.',
                textAlign: TextAlign.center,
                style: context.textTheme.bodyMedium?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: Grid.sm),
              FilledButton.icon(
                onPressed: () => ref.read(authProvider.notifier).retry(),
                icon: const Icon(LucideIcons.refreshCw),
                label: const Text('Retry'),
              ),
              const SizedBox(height: Grid.twelve),
              TextButton(
                onPressed: () => ref.read(authProvider.notifier).signOut(),
                child: Text(
                  'Remove workspace and re-pair',
                  style: TextStyle(color: context.colors.onSurfaceVariant),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
