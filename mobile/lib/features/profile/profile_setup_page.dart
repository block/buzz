import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/hosted_flow.dart';
import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
// Retain the named entry point used by the native sheet's editor engine.
// ignore: unused_import
import 'profile_avatar_sheet_app.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/onboarding_step_page.dart';
import '../../shared/widgets/onboarding_route.dart';
import 'profile_avatar_draft.dart';
import '../../shared/widgets/playing_avatar_image.dart';
import 'profile_setup_avatar_editor.dart';
import 'profile_setup_empty_avatar.dart';

part 'profile_setup_page/form.dart';

/// Profile setup for onboarding and existing-community entry, with a failure exit.
class ProfileSetupPage extends HookConsumerWidget {
  const ProfileSetupPage({super.key, required this.flow});
  final HostedFlow flow;
  static const _channel = MethodChannel('buzz/profile_setup');
  static int _nextSession = 0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(flow);
    final theme = Theme.of(context);
    final appearance = <String, Object>{
      'dark': theme.brightness == Brightness.dark,
      'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
      'containerBackgroundArgb': theme.colorScheme.surfaceContainerHighest
          .toARGB32(),
      'containerCornerRadius': Radii.container,
      'primaryArgb': theme.colorScheme.primary.toARGB32(),
      'onPrimaryArgb': theme.colorScheme.onPrimary.toARGB32(),
      'subtitleArgb': theme.colorScheme.onSurfaceVariant.toARGB32(),
    };
    final currentAppearance = useRef(appearance)..value = appearance;
    final nativeSession = useRef<int?>(null);
    useEffect(() {
      final session = ++_nextSession;
      nativeSession.value = session;
      var active = true;
      var name = '';
      final avatar = ValueNotifier<ProfileAvatarDraft?>(null);
      ModalRoute<dynamic>? profileRoute;
      ModalRoute<dynamic>? editorRoute;
      bool current() =>
          active && context.mounted && flow.stage == HostedFlowStage.profile;
      Future<void> save({bool skipAvatar = false}) async {
        if (!current()) return;
        final draft = skipAvatar ? null : avatar.value;
        final service = draft == null
            ? null
            : ref.read(mediaUploadServiceProvider);
        await flow.saveProfileDraft!(
          name.trim(),
          draft == null ? null : () => draft.upload(service!),
        );
      }

      Future<void> editAvatar() async {
        if (!current()) return;
        final result = await showBuzzModalBottomSheet<ProfileAvatarDraft>(
          context: context,
          useRootNavigator: true,
          isScrollControlled: true,
          useSafeArea: true,
          showCloseButton: false,
          padBottom: false,
          isDismissible: false,
          enableDrag: false,
          builder: (sheetContext) {
            editorRoute = ModalRoute.of(sheetContext);
            return SizedBox(
              height: MediaQuery.sizeOf(sheetContext).height * .94,
              child: ProfileSetupAvatarEditor(initialDraft: avatar.value),
            );
          },
        );
        if (!current()) return;
        editorRoute = null;
        if (result != null) avatar.value = result;
      }

      Future<void> fallback() async {
        if (!current()) return;
        await Navigator.of(context, rootNavigator: true).push<void>(
          onboardingRoute<void>(
            context,
            builder: (sheetContext) {
              profileRoute = ModalRoute.of(sheetContext);
              return _ProfileSetupForm(
                avatar: avatar,
                onName: (value) => name = value,
                editAvatar: editAvatar,
                save: save,
                notNow: flow.finishProfile,
              );
            },
          ),
        );
        if (!current()) return;
        profileRoute = null;
        if (current()) flow.finishProfile();
      }

      Future<void> present() async {
        if (defaultTargetPlatform != TargetPlatform.iOS) {
          await fallback();
          return;
        }
        _channel.setMethodCallHandler((call) async {
          if (!current() || (call.arguments as Map?)?['session'] != session) {
            return null;
          }
          switch (call.method) {
            case 'avatarDraft':
              avatar.value = decodeProfileAvatarDraft(
                (call.arguments as Map)['draft'] as Map?,
              );
              final emoji = avatar.value is ProfileUrlAvatarDraft
                  ? parseEmojiAvatarDataUrl(
                      (avatar.value as ProfileUrlAvatarDraft).url,
                    )
                  : null;
              return {
                'avatar': switch (avatar.value) {
                  ProfileImageAvatarDraft(:final bytes) => bytes,
                  ProfileAnimatedAvatarDraft(:final poster) => poster,
                  _ => null,
                },
                'emoji': emoji?.emoji,
                'emojiColor': emoji?.colorValue,
              };
            case 'save':
              name = ((call.arguments as Map)['name'] as String? ?? '').trim();
              if (name.isEmpty || name.characters.length > 100) {
                return {'saved': false};
              }
              try {
                await save(
                  skipAvatar: (call.arguments as Map)['skipAvatar'] == true,
                );
                return {'saved': true};
              } catch (_) {
                return {'saved': false};
              }
          }
          return null;
        });

        try {
          while (current()) {
            final Uint8List? bytes = switch (avatar.value) {
              ProfileImageAvatarDraft(:final bytes) => bytes,
              ProfileAnimatedAvatarDraft(:final poster) => poster,
              _ => null,
            };
            final emoji = avatar.value is ProfileUrlAvatarDraft
                ? parseEmojiAvatarDataUrl(
                    (avatar.value as ProfileUrlAvatarDraft).url,
                  )
                : null;
            final action = await _channel.invokeMethod<String>('present', {
              'session': session,
              'name': name,
              'avatar': bytes,
              'emoji': emoji?.emoji,
              'emojiColor': emoji?.colorValue,
              ...currentAppearance.value,
            });
            if (!current()) return;
            if (action != 'saved') flow.finishProfile();
            return;
          }
        } on MissingPluginException {
          await fallback();
        } on PlatformException {
          await fallback();
        } finally {
          if (active) _channel.setMethodCallHandler(null);
        }
      }

      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (current()) unawaited(present());
      });
      return () {
        active = false;
        if (nativeSession.value == session) nativeSession.value = null;
        if (defaultTargetPlatform == TargetPlatform.iOS) {
          _channel.setMethodCallHandler(null);
          unawaited(
            _channel
                .invokeMethod<void>('dismiss', {'session': session})
                .catchError((Object _) {}),
          );
        }
        final routes = [editorRoute, profileRoute];
        avatar.dispose();
        WidgetsBinding.instance.addPostFrameCallback((_) {
          for (final route in routes) {
            if (route != null && route.isActive) {
              route.navigator?.removeRoute(route);
            }
          }
        });
      };
    }, [flow, flow.stage == HostedFlowStage.profile]);
    useEffect(() {
      if (defaultTargetPlatform != TargetPlatform.iOS) return null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        final session = nativeSession.value;
        if (!context.mounted || session == null) return;
        unawaited(
          _channel
              .invokeMethod<void>('appearance', {
                'session': session,
                ...currentAppearance.value,
              })
              .catchError((Object _) {}),
        );
      });
      return null;
    }, [theme]);
    return const SizedBox.shrink();
  }
}
