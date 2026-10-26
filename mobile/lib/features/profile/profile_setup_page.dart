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
import 'profile_avatar_draft.dart';
import '../../shared/widgets/playing_avatar_image.dart';
import 'profile_setup_avatar_editor.dart';

/// One optional profile sheet for onboarding and existing-community entry.
class ProfileSetupPage extends HookConsumerWidget {
  const ProfileSetupPage({super.key, required this.flow});
  final HostedFlow flow;
  static const _channel = MethodChannel('buzz/profile_setup');
  static int _nextSession = 0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = utilitySurfaceThemeData(Theme.of(context));
    useEffect(() {
      final session = ++_nextSession;
      var active = true;
      var name = '';
      final avatar = ValueNotifier<ProfileAvatarDraft?>(null);
      ModalRoute<dynamic>? profileRoute;
      ModalRoute<dynamic>? editorRoute;
      bool current() =>
          active && context.mounted && flow.stage == HostedFlowStage.profile;
      Future<void> save() async {
        if (!current()) return;
        final draft = avatar.value;
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
        await showBuzzModalBottomSheet<void>(
          context: context,
          title: 'Set up your profile',
          isDismissible: false,
          enableDrag: false,
          showCloseButton: false,
          useRootNavigator: true,
          isScrollControlled: true,
          useSafeArea: true,
          builder: (sheetContext) {
            profileRoute = ModalRoute.of(sheetContext);
            return _ProfileSetupForm(
              avatar: avatar,
              onName: (value) => name = value,
              editAvatar: editAvatar,
              save: save,
              skip: flow.finishProfile,
            );
          },
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
                await save();
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
              'dark': theme.brightness == Brightness.dark,
              'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
              'containerBackgroundArgb': theme
                  .colorScheme
                  .surfaceContainerHighest
                  .toARGB32(),
              'containerCornerRadius': Radii.container,

              'primaryArgb': theme.colorScheme.primary.toARGB32(),
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
    }, [flow]);
    return const SizedBox.shrink();
  }
}

class _ProfileSetupForm extends HookWidget {
  const _ProfileSetupForm({
    required this.avatar,
    required this.onName,
    required this.editAvatar,
    required this.save,
    required this.skip,
  });
  final ValueNotifier<ProfileAvatarDraft?> avatar;
  final ValueChanged<String> onName;
  final Future<void> Function() editAvatar;
  final Future<void> Function() save;
  final VoidCallback skip;
  @override
  Widget build(BuildContext context) {
    final draft = useValueListenable(avatar);
    final name = useTextEditingController();
    useListenable(name);
    final busy = useState(false);
    final error = useState(false);
    return PopScope(
      canPop: !busy.value,
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          20,
          16,
          20,
          MediaQuery.viewInsetsOf(context).bottom + 20,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            IconButton(
              onPressed: busy.value ? null : editAvatar,
              icon: switch (draft) {
                ProfileImageAvatarDraft(:final bytes) => CircleAvatar(
                  radius: 32,
                  backgroundImage: MemoryImage(bytes),
                ),
                ProfileAnimatedAvatarDraft(:final poster) => CircleAvatar(
                  radius: 32,
                  backgroundImage: MemoryImage(poster),
                ),
                ProfileUrlAvatarDraft(:final url) => PlayingAvatarImage(
                  imageUrl: url,
                  radius: 32,
                  fallback: const Icon(Icons.add_a_photo_outlined),
                ),
                null => const Icon(Icons.add_a_photo_outlined),
              },
              iconSize: 64,
              tooltip: 'Add profile photo',
            ),
            TextField(
              controller: name,
              maxLength: 100,
              enabled: !busy.value,
              onChanged: onName,
              decoration: const InputDecoration(labelText: 'Display name'),
            ),
            if (error.value)
              const Text('Could not save your profile. Try again or skip.'),
            FilledButton(
              style: FilledButton.styleFrom(
                minimumSize: const Size.fromHeight(52),
                shape: const StadiumBorder(),
              ),
              onPressed: busy.value || name.text.trim().isEmpty
                  ? null
                  : () async {
                      busy.value = true;
                      error.value = false;
                      try {
                        await save();
                      } catch (_) {
                        if (context.mounted) error.value = true;
                      } finally {
                        if (context.mounted) busy.value = false;
                      }
                    },
              child: Text(busy.value ? 'Saving…' : 'Continue'),
            ),
            OutlinedButton(
              style: OutlinedButton.styleFrom(
                minimumSize: const Size.fromHeight(52),
                shape: const StadiumBorder(),
              ),
              onPressed: busy.value ? null : skip,
              child: const Text('Skip'),
            ),
          ],
        ),
      ),
    );
  }
}
