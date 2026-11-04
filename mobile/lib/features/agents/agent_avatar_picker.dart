import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/agent_avatar_catalog.dart';
import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/emoji/emoji_picker.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_shape.dart';
import '../../shared/widgets/preset_agent_avatar.dart';
import '../../shared/widgets/profile_avatar_crop_page.dart';
import 'agent_avatar_category.dart';
import 'agent_avatar_choices.dart';
import 'agent_avatar_geometry.dart';
import 'agent_avatar_hero.dart';
import 'agent_category_motion.dart';
import 'agent_category_transition.dart';
import 'agent_character_artwork.dart';
import 'agent_character_stage.dart';
import 'agent_emoji_stage.dart';

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

/// Renders an emoji on a solid [color] to the 512px PNG that creation uploads.
Future<Uint8List?> renderEmojiAvatarPng(String emoji, int color) async {
  // Export pixels, as desktop does: registry pictures are ordinary HTTPS
  // media URLs, not client-specific SVG data URLs.
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  canvas.drawColor(Color(color), BlendMode.src);
  final text = TextPainter(
    text: TextSpan(text: emoji, style: const TextStyle(fontSize: 258)),
    textDirection: TextDirection.ltr,
  )..layout();
  text.paint(canvas, Offset((512 - text.width) / 2, (512 - text.height) / 2));
  final picture = recorder.endRecording();
  final raster = await picture.toImage(512, 512);
  final data = await raster.toByteData(format: ui.ImageByteFormat.png);
  raster.dispose();
  picture.dispose();
  return data?.buffer.asUint8List();
}

/// Avatar preview with character, shared emoji, and image selection controls.
class AgentAvatarPicker extends HookConsumerWidget {
  const AgentAvatarPicker({
    super.key,
    required this.selection,
    this.heroTag,
    required this.onChanged,
    required this.disabled,
    required this.onBusyChanged,
    this.controlsHeight = 320,
  });

  /// Height available for the scrolling choices below the preview.
  final double controlsHeight;
  final AgentAvatarSelection selection;
  final Object? heroTag;
  final ValueChanged<AgentAvatarSelection> onChanged;
  final bool disabled;
  final ValueChanged<bool> onBusyChanged;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final initialIndex = useMemoized(() {
      final found = agentAvatarCatalog.indexWhere(
        (item) => item.url == selection.url,
      );
      return found < 0 ? 0 : found;
    }, const []);
    final index = useState(initialIndex);
    final pack = useState(
      AgentAvatarPack.values.firstWhere(
        (pack) => pack.indices.contains(initialIndex),
      ),
    );
    final category = useState(
      selection.url != null
          ? AgentAvatarCategory.pack(pack.value)
          : selection.emoji != null
          ? AgentAvatarCategory.emoji
          : AgentAvatarCategory.image,
    );
    final packSelections = useRef(<AgentAvatarPack, int>{
      pack.value: initialIndex,
    });
    final examples = useMemoized(
      () => {
        for (final pack in AgentAvatarPack.values)
          pack: (pack.indices.toList()..shuffle()).first,
      },
      const [],
    );
    final emoji = useState(selection.emoji ?? '😊');
    final color = useState(selection.color ?? emojiAvatarColors.first);
    final image = useState<AgentAvatarSelection?>(
      selection.bytes != null && selection.emoji == null ? selection : null,
    );
    final emojiSelection = useState<AgentAvatarSelection?>(
      selection.emoji != null ? selection : null,
    );
    final error = useState<String?>(null);
    final busy = useState(false);
    final locked = disabled || busy.value;

    Future<void> selectEmoji() async {
      final data = await renderEmojiAvatarPng(emoji.value, color.value);
      if (context.mounted && data != null) {
        final rendered = AgentAvatarSelection.emoji(
          data,
          emoji.value,
          color.value,
        );
        emojiSelection.value = rendered;
        onChanged(rendered);
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

    Future<void> applyImage(Uint8List bytes) async {
      // Crop exports JPEG; convert to PNG so all local drafts share one MIME.
      final codec = await ui.instantiateImageCodec(bytes);
      final frame = await codec.getNextFrame();
      final data = await frame.image.toByteData(format: ui.ImageByteFormat.png);
      frame.image.dispose();
      codec.dispose();
      if (!context.mounted || data == null) return;
      image.value = AgentAvatarSelection.image(data.buffer.asUint8List());
      category.value = AgentAvatarCategory.image;
      onChanged(image.value!);
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
        await applyImage(cropped);
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

    final imageHeight = controlsHeight + agentAvatarPreviewSpace;
    Widget buildPreview(
      AgentAvatarSelection selection,
      AgentAvatarCategory previewCategory,
    ) {
      final previewSize = previewCategory.pack != null
          ? math.min(
              agentAvatarMaxPreviewSize,
              imageHeight - agentAvatarTraySpace,
            )
          : 220.0;
      return AgentAvatarHero(
        tag: heroTag,
        selection: selection,
        verticalOffset: agentCharacterOffset(selection.url, scale: 0.85),
        child: Semantics(
          label: selection.url == null
              ? 'Selected agent avatar'
              : 'Selected avatar: ${agentAvatarCatalog.firstWhere((item) => item.url == selection.url).name}',
          liveRegion: true,
          image: true,
          child: SizedBox.square(
            dimension: previewSize,
            child: selection.bytes != null
                ? AvatarClip(
                    isAgent: true,
                    child: AnimatedSwitcher(
                      transitionBuilder: (child, animation) => AnimatedBuilder(
                        animation: animation,
                        child: child,
                        builder: (context, child) => Opacity(
                          // Keep the old bitmap opaque beneath the incoming one
                          // so the unchanged emoji does not dim during the blend.
                          opacity: animation.status == AnimationStatus.reverse
                              ? 1
                              : animation.value,
                          child: child,
                        ),
                      ),
                      duration:
                          MediaQuery.disableAnimationsOf(context) ||
                              previewCategory != AgentAvatarCategory.emoji
                          ? Duration.zero
                          : const Duration(milliseconds: 180),
                      child: Image.memory(
                        selection.bytes!,
                        key: ValueKey((selection.emoji, selection.color)),
                        fit: BoxFit.cover,
                      ),
                    ),
                  )
                : AgentCharacterAlignment(
                    url: selection.url!,
                    scale: 0.85,
                    child: PresetAgentAvatar(
                      imageUrl: selection.url!,
                      posterPlaceholder: previewCategory.pack != null
                          ? characterPoster(selection.url!)
                          : null,
                      size: previewSize,
                      fallback: const Icon(Icons.smart_toy_outlined, size: 80),
                    ),
                  ),
          ),
        ),
      );
    }

    final selectedCategory = category.value;
    final position = selectedCategory.position;
    final previousCategory = useRef(position);
    final slideDirection = useRef(1.0);
    if (previousCategory.value != position) {
      slideDirection.value = position > previousCategory.value ? 1 : -1;
      previousCategory.value = position;
    }
    final direction = slideDirection.value;

    return Column(
      children: [
        SizedBox(
          height: imageHeight,
          child: AnimatedSwitcher(
            duration: MediaQuery.disableAnimationsOf(context)
                ? Duration.zero
                : const Duration(milliseconds: 240),
            switchInCurve: Curves.easeOutCubic,
            switchOutCurve: Curves.easeInCubic,
            transitionBuilder: (child, animation) => AnimatedBuilder(
              animation: animation,
              child: child,
              builder: (context, child) {
                final outgoing = animation.status == AnimationStatus.reverse;
                return AgentCategoryTransition(
                  animation: animation,
                  direction: direction,
                  child: HeroMode(
                    enabled: !outgoing,
                    child: IgnorePointer(ignoring: outgoing, child: child),
                  ),
                );
              },
            ),
            child: SizedBox(
              key: ValueKey(selectedCategory),
              height: imageHeight,
              child: Builder(
                builder: (context) {
                  final page = selectedCategory;
                  final pagePack = page.pack;
                  if (pagePack != null) {
                    final selected = pagePack == pack.value
                        ? index.value
                        : packSelections.value[pagePack] ?? examples[pagePack]!;
                    final draft = AgentAvatarSelection.url(
                      agentAvatarCatalog[selected].url,
                    );
                    return AgentCharacterStage(
                      key: ValueKey(pagePack),
                      pack: pagePack,
                      selected: selected,
                      height: imageHeight,
                      preview: IgnorePointer(child: buildPreview(draft, page)),
                      disabled: locked,
                      onSelected: (value) {
                        index.value = value;
                        packSelections.value[pagePack] = value;
                        onChanged(
                          AgentAvatarSelection.url(
                            agentAvatarCatalog[value].url,
                          ),
                        );
                      },
                    );
                  }
                  if (page == AgentAvatarCategory.emoji) {
                    final draft = emojiSelection.value;
                    return AgentEmojiStage(
                      emoji: draft?.emoji,
                      height: imageHeight,
                      preview: draft != null
                          ? buildPreview(draft, page)
                          : SizedBox.square(
                              dimension: 220,
                              child: AvatarClip(
                                isAgent: true,
                                child: ColoredBox(
                                  color: Color(color.value),
                                  child: Center(
                                    child: Text(
                                      emoji.value,
                                      style: const TextStyle(fontSize: 110),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                      selectedColor: color.value,
                      disabled: locked,
                      onChooseEmoji: () => showEmojiPicker(
                        context: context,
                        nativeEmojiOnly: true,
                        onSelect: (value) async {
                          if (!context.mounted) return;
                          emoji.value = value;
                          await renderEmoji();
                        },
                      ),
                      onColorSelected: (value) async {
                        color.value = value;
                        await renderEmoji();
                      },
                    );
                  }
                  final preview = image.value != null
                      ? buildPreview(image.value!, page)
                      : const SizedBox.square(
                          dimension: 220,
                          child: Center(
                            child: Icon(Icons.image_outlined, size: 80),
                          ),
                        );
                  return Stack(
                    children: [
                      Positioned.fill(
                        bottom: agentAvatarTraySpace,
                        child: AgentCategoryMotion(
                          child: Center(child: preview),
                        ),
                      ),
                    ],
                  );
                },
              ),
            ),
          ),
        ),
        const SizedBox(height: Grid.sm),
        AgentAvatarChoices(
          category: category.value,
          examples: examples,
          emoji: emoji.value,
          disabled: locked,
          onPack: (next) {
            packSelections.value[pack.value] = index.value;
            pack.value = next;
            index.value = packSelections.value[next] ?? examples[next]!;
            category.value = AgentAvatarCategory.pack(next);
            error.value = null;
            onChanged(
              AgentAvatarSelection.url(agentAvatarCatalog[index.value].url),
            );
          },
          onEmoji: () {
            category.value = AgentAvatarCategory.emoji;
            error.value = null;
            renderEmoji();
          },
          onImage: pickImage,
        ),
        if (busy.value && category.value != AgentAvatarCategory.emoji)
          const LinearProgressIndicator(),
        if (error.value != null)
          Text(error.value!, style: TextStyle(color: context.colors.error)),
      ],
    );
  }
}
