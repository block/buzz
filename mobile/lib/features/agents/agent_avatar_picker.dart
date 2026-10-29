import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import '../../shared/widgets/ios_native_segmented_control.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/agent_avatar_catalog.dart';
import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/emoji_avatar_controls.dart';
import '../../shared/widgets/preset_agent_avatar.dart';
import '../../shared/widgets/profile_avatar_crop_page.dart';

/// An avatar kept locally until creation uploads it to the selected community.
class AgentAvatarSelection {
  AgentAvatarSelection.url(this.url) : bytes = null, emoji = null, color = null;
  AgentAvatarSelection.image(this.bytes)
    : url = null,
      emoji = null,
      color = null;
  AgentAvatarSelection.emoji(this.bytes, this.emoji, this.color) : url = null;
  final String? emoji;
  final int? color;
  final String? url;
  final Uint8List? bytes;
  Future<String>? _uploaded;
  MediaUploadService? _service;

  Future<String> prepare(MediaUploadService service) async {
    if (url != null) return url!;
    if (!identical(service, _service)) {
      _service = service;
      _uploaded = null;
    }
    final upload = _uploaded ??= service
        .uploadBytes(bytes!, mimeType: 'image/png')
        .then((value) => value.url);
    try {
      return await upload;
    } catch (_) {
      if (identical(_uploaded, upload)) _uploaded = null;
      rethrow;
    }
  }
}

/// Avatar preview with character, shared emoji, and image selection controls.
class AgentAvatarPicker extends HookConsumerWidget {
  const AgentAvatarPicker({
    super.key,
    required this.selection,
    required this.onChanged,
    required this.disabled,
    required this.onBusyChanged,
  });
  final AgentAvatarSelection selection;
  final ValueChanged<AgentAvatarSelection> onChanged;
  final bool disabled;
  final ValueChanged<bool> onBusyChanged;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mode = useState(
      selection.url != null
          ? 0
          : selection.emoji != null
          ? 1
          : 2,
    );
    final initialIndex = useMemoized(() {
      final found = agentAvatarCatalog.indexWhere(
        (item) => item.url == selection.url,
      );
      return found < 0 ? 0 : found;
    }, const []);
    final index = useState(initialIndex);
    final emoji = useState(selection.emoji ?? '😊');
    final color = useState(selection.color ?? emojiAvatarColors.first);
    final image = useState<AgentAvatarSelection?>(
      selection.bytes != null && selection.emoji == null ? selection : null,
    );
    final error = useState<String?>(null);
    final busy = useState(false);
    final emojiSection = useState(EmojiEditorSection.emoji);
    final dataset = ref.watch(emojiDatasetProvider);
    final locked = disabled || busy.value;

    Future<void> selectEmoji() async {
      // Export pixels, as desktop does: registry pictures are ordinary HTTPS
      // media URLs, not client-specific SVG data URLs.
      final recorder = ui.PictureRecorder();
      final canvas = Canvas(recorder);
      canvas.drawColor(Color(color.value), BlendMode.src);
      final text = TextPainter(
        text: TextSpan(
          text: emoji.value,
          style: const TextStyle(fontSize: 258),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      text.paint(
        canvas,
        Offset((512 - text.width) / 2, (512 - text.height) / 2),
      );
      final picture = recorder.endRecording();
      final raster = await picture.toImage(512, 512);
      final data = await raster.toByteData(format: ui.ImageByteFormat.png);
      raster.dispose();
      picture.dispose();
      if (context.mounted && data != null) {
        onChanged(
          AgentAvatarSelection.emoji(
            data.buffer.asUint8List(),
            emoji.value,
            color.value,
          ),
        );
      }
    }

    Future<void> renderEmoji() async {
      busy.value = true;
      onBusyChanged(true);
      try {
        await selectEmoji();
      } catch (_) {
        if (context.mounted) {
          error.value = 'Could not prepare the emoji. Try again.';
        }
      } finally {
        if (context.mounted) {
          busy.value = false;
          onBusyChanged(false);
        }
      }
    }

    Future<void> pickImage() async {
      busy.value = true;
      onBusyChanged(true);
      error.value = null;
      try {
        final service = ref.read(mediaUploadServiceProvider);
        final picked = await service.pickGalleryImage();
        if (picked == null || !context.mounted) return;
        final prepared = await service.prepareImageBytes(picked);
        if (!context.mounted) return;
        final cropped = await Navigator.of(context).push<Uint8List>(
          MaterialPageRoute(
            fullscreenDialog: true,
            builder: (_) =>
                ProfileAvatarCropPage(imageBytes: Future.value(prepared)),
          ),
        );
        if (cropped == null || !context.mounted) return;
        // Crop exports JPEG; convert to PNG so all local drafts share one MIME.
        final codec = await ui.instantiateImageCodec(cropped);
        final frame = await codec.getNextFrame();
        final data = await frame.image.toByteData(
          format: ui.ImageByteFormat.png,
        );
        frame.image.dispose();
        codec.dispose();
        if (!context.mounted || data == null) return;
        image.value = AgentAvatarSelection.image(data.buffer.asUint8List());
        mode.value = 2;
        onChanged(image.value!);
      } catch (_) {
        if (context.mounted) {
          error.value = 'Could not prepare that image. Try again.';
        }
      } finally {
        if (context.mounted) {
          busy.value = false;
          onBusyChanged(false);
        }
      }
    }

    Future<void> selectMode(int next) async {
      if (locked) return;
      if (next == 2 && image.value == null) {
        await pickImage();
        return;
      }
      mode.value = next;
      error.value = null;
      if (next == 0) {
        onChanged(
          AgentAvatarSelection.url(agentAvatarCatalog[index.value].url),
        );
      }
      if (next == 1) await renderEmoji();
      if (next == 2) onChanged(image.value!);
    }

    return Column(
      children: [
        if (defaultTargetPlatform == TargetPlatform.iOS)
          IosNativeSegmentedControl(
            items: const ['Gloopies', 'Emoji', 'Image'],
            selectedIndex: mode.value,
            onChanged: locked ? null : selectMode,
          )
        else
          SegmentedButton<int>(
            segments: const [
              ButtonSegment(value: 0, label: Text('Gloopies')),
              ButtonSegment(value: 1, label: Text('Emoji')),
              ButtonSegment(value: 2, label: Text('Image')),
            ],
            selected: {mode.value},
            showSelectedIcon: false,
            onSelectionChanged: locked
                ? null
                : (values) => selectMode(values.first),
          ),
        const SizedBox(height: Grid.sm),
        Semantics(
          label: 'Selected agent avatar',
          image: true,
          child: SizedBox.square(
            dimension: 220,
            child: selection.bytes != null
                ? ClipOval(
                    child: Image.memory(selection.bytes!, fit: BoxFit.cover),
                  )
                : PresetAgentAvatar(
                    imageUrl: selection.url!,
                    size: 220,
                    fallback: const Icon(Icons.smart_toy_outlined, size: 80),
                  ),
          ),
        ),
        const SizedBox(height: Grid.sm),
        if (mode.value == 0)
          SizedBox(
            height: 320,
            child: GridView.builder(
              key: const ValueKey('agent-avatar-grid'),
              gridDelegate: const SliverGridDelegateWithFixedCrossAxisCount(
                crossAxisCount: 4,
              ),
              itemCount: agentAvatarCatalog.length,
              itemBuilder: (context, i) => Semantics(
                label: agentAvatarCatalog[i].name,
                selected: index.value == i,
                button: true,
                child: InkWell(
                  key: ValueKey('agent-avatar-$i'),
                  onTap: locked
                      ? null
                      : () {
                          index.value = i;
                          onChanged(
                            AgentAvatarSelection.url(agentAvatarCatalog[i].url),
                          );
                        },
                  borderRadius: BorderRadius.circular(Radii.container),
                  child: Container(
                    margin: const EdgeInsets.all(Grid.xxs),
                    padding: const EdgeInsets.all(Grid.xxs),
                    decoration: BoxDecoration(
                      borderRadius: BorderRadius.circular(Radii.container),
                      border: Border.all(
                        color: index.value == i
                            ? context.colors.primary
                            : Colors.transparent,
                        width: 2,
                      ),
                    ),
                    child: AvatarImageContent(
                      imageUrl: agentAvatarCatalog[i].url,
                      fallback: const Icon(Icons.smart_toy_outlined),
                    ),
                  ),
                ),
              ),
            ),
          ),
        if (mode.value == 1)
          dataset.when(
            loading: () => const SizedBox(
              height: 320,
              child: Center(child: CircularProgressIndicator()),
            ),
            error: (_, _) => TextButton(
              onPressed: () => ref.invalidate(emojiDatasetProvider),
              child: const Text('Retry emoji'),
            ),
            data: (data) => IgnorePointer(
              ignoring: locked,
              child: EmojiAvatarControls(
                height: 320,
                activeSection: emojiSection.value,
                dataset: data,
                selectedEmoji: emoji.value,
                selectedColor: color.value,
                transitionProgress: 1,
                transitionDirection: 0,
                onSectionChanged: (value) => emojiSection.value = value,
                onEmojiSelected: (value) async {
                  emoji.value = value;
                  await renderEmoji();
                },
                onColorSelected: (value) async {
                  color.value = value;
                  await renderEmoji();
                },
              ),
            ),
          ),
        if (mode.value == 2)
          TextButton.icon(
            onPressed: locked ? null : pickImage,
            icon: const Icon(Icons.photo_library_outlined),
            label: const Text('Choose image'),
          ),
        if (busy.value) const LinearProgressIndicator(),
        if (error.value != null)
          Text(error.value!, style: TextStyle(color: context.colors.error)),
      ],
    );
  }
}
