part of '../compose_bar.dart';

bool _rejectsNonVoiceAttachment(
  _ComposerVoiceNote voiceNote,
  List<_PendingAttachment> attachments,
  ValueNotifier<String?> uploadError,
) {
  if (!voiceNote.isPreparing &&
      voiceNote.recorder == null &&
      !attachments.any(
        (attachment) => attachment.kind == _PendingAttachmentKind.voiceNote,
      )) {
    return false;
  }
  uploadError.value = voiceNote.recorder != null || voiceNote.isPreparing
      ? 'Finish or discard the voice note before attaching a file.'
      : 'A voice note must be the only attachment.';
  return true;
}

bool _queueComposerAttachment(
  XFile file,
  _PendingAttachmentKind kind,
  ObjectRef<_ComposerVoiceNote> voiceNote,
  ValueNotifier<List<_PendingAttachment>> attachments,
  ValueNotifier<String?> uploadError,
  ObjectRef<int> draftRevision, {
  bool deleteAfterUse = false,
}) {
  if (kind != _PendingAttachmentKind.voiceNote &&
      _rejectsNonVoiceAttachment(
        voiceNote.value,
        attachments.value,
        uploadError,
      )) {
    return false;
  }
  draftRevision.value += 1;
  uploadError.value = null;
  attachments.value = [
    ...attachments.value,
    _PendingAttachment(file: file, kind: kind, deleteAfterUse: deleteAfterUse),
  ];
  return true;
}

bool _queueComposerImages(
  List<XFile> images,
  _ComposerVoiceNote voiceNote,
  ValueNotifier<List<_PendingAttachment>> attachments,
  ValueNotifier<String?> uploadError,
  ObjectRef<int> draftRevision,
  bool deleteAfterUse,
) {
  if (images.isEmpty ||
      _rejectsNonVoiceAttachment(voiceNote, attachments.value, uploadError)) {
    return false;
  }
  draftRevision.value += 1;
  uploadError.value = null;
  attachments.value = [
    ...attachments.value,
    for (final image in images)
      _PendingAttachment(
        file: image,
        kind: _PendingAttachmentKind.image,
        deleteAfterUse: deleteAfterUse,
      ),
  ];
  return true;
}

enum _AttachmentSurface { closed, menu, camera, photos }

const _attachmentMenuWidth = 216.0;
const _attachmentMenuPadding = Grid.xs;
const _attachmentMenuItemHeight = 52.0;
const _attachmentMenuItemSpacing = Grid.xxs;
const _attachmentMenuIconSize = 24.0;
const _attachmentMenuIconSlotWidth = 28.0;
const _attachmentExpandedHeight = 372.0;

@immutable
class _AttachmentMenuLayout {
  final double itemHeight;
  final double contentHeight;
  final double height;

  const _AttachmentMenuLayout({
    required this.itemHeight,
    required this.contentHeight,
    required this.height,
  });

  factory _AttachmentMenuLayout.from(BuildContext context) {
    final textPainter = TextPainter(
      text: TextSpan(text: 'Camera', style: context.textTheme.titleMedium),
      textDirection: Directionality.of(context),
      textScaler: MediaQuery.textScalerOf(context),
      maxLines: 1,
    )..layout();
    final itemHeight = math.max(
      _attachmentMenuItemHeight,
      textPainter.height + (Grid.xxs * 2),
    );
    textPainter.dispose();
    final contentHeight =
        (_attachmentMenuPadding * 2) +
        (itemHeight * 5) +
        (_attachmentMenuItemSpacing * 4);

    return _AttachmentMenuLayout(
      itemHeight: itemHeight,
      contentHeight: contentHeight,
      height: math.min(contentHeight, _attachmentExpandedHeight),
    );
  }

  bool get isScrollable => contentHeight > height;
}

class _AttachmentSurfacePanel extends HookWidget {
  final _AttachmentSurface surface;
  final Widget suggestionPanel;
  final VoidCallback onBack;
  final VoidCallback onCamera;
  final VoidCallback onPhotos;
  final VoidCallback onVideo;
  final VoidCallback onVoiceNote;
  final VoidCallback onFiles;
  final Future<void> Function(XFile image) onCapture;
  final Future<List<XFile>> Function() onPickAllPhotos;
  final Future<void> Function(List<XFile> photos) onChoosePhotos;
  final Future<void> Function(List<XFile> photos) onChooseAllPhotos;

  const _AttachmentSurfacePanel({
    super.key,
    required this.surface,
    required this.suggestionPanel,
    required this.onBack,
    required this.onCamera,
    required this.onPhotos,
    required this.onVideo,
    required this.onVoiceNote,
    required this.onFiles,
    required this.onCapture,
    required this.onPickAllPhotos,
    required this.onChoosePhotos,
    required this.onChooseAllPhotos,
  });

  @override
  Widget build(BuildContext context) {
    if (surface == _AttachmentSurface.closed) return suggestionPanel;

    final menuLayout = _AttachmentMenuLayout.from(context);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final isExpanded =
        surface == _AttachmentSurface.camera ||
        surface == _AttachmentSurface.photos;
    final morphController = useAnimationController(
      initialValue: isExpanded ? 1 : 0,
      duration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 320),
      reverseDuration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 250),
    );
    final rawProgress = useAnimation(morphController);
    final renderedExpandedSurface = useState<_AttachmentSurface?>(
      isExpanded ? surface : null,
    );
    final latestSurface = useRef(surface);
    latestSurface.value = surface;

    useEffect(() {
      void disposeCollapsedContent(AnimationStatus status) {
        if (status == AnimationStatus.dismissed &&
            latestSurface.value == _AttachmentSurface.menu) {
          renderedExpandedSurface.value = null;
        }
      }

      morphController.addStatusListener(disposeCollapsedContent);
      return () =>
          morphController.removeStatusListener(disposeCollapsedContent);
    }, [morphController]);

    useEffect(() {
      if (isExpanded) {
        renderedExpandedSurface.value = surface;
        morphController.forward();
      } else {
        morphController.reverse();
      }
      return null;
    }, [isExpanded, morphController, surface]);

    final visibleExpandedSurface =
        renderedExpandedSurface.value ?? (isExpanded ? surface : null);
    final cameraInitializationReady =
        reducedMotion ||
        (surface == _AttachmentSurface.camera && rawProgress >= 1);
    final expandedContent = switch (visibleExpandedSurface) {
      _AttachmentSurface.camera => KeyedSubtree(
        key: const ValueKey('camera-preview'),
        child: KeyedSubtree(
          key: ValueKey(
            cameraInitializationReady
                ? 'camera-initialization-ready'
                : 'camera-initialization-deferred',
          ),
          child: _InlineCameraPreview(
            initializeCamera: cameraInitializationReady,
            onClose: onBack,
            onCapture: onCapture,
          ),
        ),
      ),
      _AttachmentSurface.photos => KeyedSubtree(
        key: const ValueKey('photo-gallery'),
        child: _PhotoGalleryPicker(
          onBack: onBack,
          onPickAllPhotos: onPickAllPhotos,
          onChoosePhotos: onChoosePhotos,
          onChooseAllPhotos: onChooseAllPhotos,
        ),
      ),
      _AttachmentSurface.closed ||
      _AttachmentSurface.menu ||
      null => const SizedBox.shrink(),
    };

    double interval(double value, double begin, double end) {
      return ((value - begin) / (end - begin)).clamp(0.0, 1.0);
    }

    final menuOpacity = 1 - interval(rawProgress, 0.12, 0.38);
    final expandedOpacity = interval(rawProgress, 0.28, 0.65);
    final sizeProgress = morphController.status == AnimationStatus.reverse
        ? const Cubic(0.22, 1, 0.36, 1).transform(rawProgress)
        : Curves.easeInOutCubic.transform(rawProgress);

    return LayoutBuilder(
      builder: (context, constraints) {
        final expandedWidth = constraints.maxWidth;
        const expandedHeight = _attachmentExpandedHeight;
        final width =
            _attachmentMenuWidth +
            ((expandedWidth - _attachmentMenuWidth) * sizeProgress);
        final height =
            menuLayout.height +
            ((expandedHeight - menuLayout.height) * sizeProgress);
        final baseColor = appPopoverColor(context);
        final expandedColor =
            visibleExpandedSurface == _AttachmentSurface.camera
            ? Colors.black
            : baseColor;

        return Align(
          alignment: Alignment.topLeft,
          heightFactor: 1,
          child: SizedBox(
            width: width,
            height: height,
            child: Material(
              key: const ValueKey('attachment-surface-popover'),
              type: MaterialType.card,
              color: Color.lerp(baseColor, expandedColor, sizeProgress),
              surfaceTintColor: Colors.transparent,
              elevation: appPopoverElevation,
              shadowColor: appPopoverShadowColor(context),
              shape: appPopoverShape(context),
              clipBehavior: Clip.antiAlias,
              child: Stack(
                clipBehavior: Clip.hardEdge,
                children: [
                  Positioned(
                    left: 0,
                    top: 0,
                    width: _attachmentMenuWidth,
                    height: menuLayout.height,
                    child: IgnorePointer(
                      ignoring: surface != _AttachmentSurface.menu,
                      child: Opacity(
                        opacity: menuOpacity,
                        child: _AttachmentMenu(
                          layout: menuLayout,
                          onCamera: onCamera,
                          onPhotos: onPhotos,
                          onVideo: onVideo,
                          onVoiceNote: onVoiceNote,
                          onFiles: onFiles,
                        ),
                      ),
                    ),
                  ),
                  Positioned(
                    left: 0,
                    top: 0,
                    width: expandedWidth,
                    height: expandedHeight,
                    child: IgnorePointer(
                      ignoring: !isExpanded,
                      child: Opacity(
                        opacity: expandedOpacity,
                        child: expandedContent,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

@immutable
class _ComposeDraftPayload {
  final String content;
  final List<List<String>> mediaTags;

  const _ComposeDraftPayload({required this.content, required this.mediaTags});

  factory _ComposeDraftPayload.fromDraft({
    required String text,
    required List<BlobDescriptor> attachments,
    required List<CustomEmoji> customEmoji,
  }) {
    var content = text;
    final mediaTags = <List<String>>[];
    for (final attachment in attachments) {
      mediaTags.add(attachment.toImetaTag());
      content += '\n${attachment.toMarkdownImage()}';
    }
    mediaTags.addAll(buildCustomEmojiTags(content, customEmoji));
    return _ComposeDraftPayload(content: content, mediaTags: mediaTags);
  }
}

enum _PendingAttachmentKind { image, video, voiceNote, file }

@immutable
class _PendingAttachment {
  static var _nextId = 0;

  final int id;
  final XFile file;
  final _PendingAttachmentKind kind;
  final bool deleteAfterUse;
  final Duration? duration;
  final List<double> waveform;

  _PendingAttachment({
    required this.file,
    required this.kind,
    this.deleteAfterUse = false,
    this.duration,
    this.waveform = const [],
  }) : id = _nextId++;
}

Future<void> _deleteXFile(XFile file) async {
  final path = file.path;
  if (path.isEmpty) return;
  try {
    await File(path).delete();
  } on FileSystemException {
    // Temporary files may already have been removed by the operating system.
  }
}

Future<void> _deleteOwnedAttachments(
  Iterable<_PendingAttachment> attachments,
) async {
  for (final attachment in attachments) {
    if (attachment.deleteAfterUse) await _deleteXFile(attachment.file);
  }
}

void _useOwnedAttachmentCleanup(
  ValueNotifier<List<_PendingAttachment>> attachments,
) {
  useEffect(
    () =>
        () => unawaited(_deleteOwnedAttachments(attachments.value)),
    [attachments],
  );
}

void _removePendingAttachment(
  ValueNotifier<List<_PendingAttachment>> attachments,
  ObjectRef<int> draftRevision,
  int id,
) {
  draftRevision.value += 1;
  final removed = attachments.value.where((attachment) => attachment.id == id);
  attachments.value = _withoutAttachment(attachments.value, id);
  unawaited(_deleteOwnedAttachments(removed));
}

Future<void> _retainAndQueueImages(
  BuildContext context,
  List<XFile> images,
  bool Function(List<XFile>, {bool deleteAfterUse}) queueImages,
) async {
  final retained = await retainTemporaryImages(images);
  if (!context.mounted || !queueImages(retained, deleteAfterUse: true)) {
    for (final image in retained) {
      await _deleteXFile(image);
    }
  }
}

Future<BlobDescriptor> _uploadPendingAttachment(
  MediaUploadService service,
  _PendingAttachment attachment, {
  ValueChanged<double>? onProgress,
  UploadCancellationToken? cancellationToken,
}) => switch (attachment.kind) {
  _PendingAttachmentKind.image => service.uploadImage(
    attachment.file,
    onProgress: onProgress,
    cancellationToken: cancellationToken,
  ),
  _PendingAttachmentKind.video => service.uploadVideo(
    attachment.file,
    onProgress: onProgress,
    cancellationToken: cancellationToken,
  ),
  _PendingAttachmentKind.voiceNote => service.uploadVoiceNote(
    attachment.file,
    duration: attachment.duration ?? Duration.zero,
    onProgress: onProgress,
    cancellationToken: cancellationToken,
  ),
  _PendingAttachmentKind.file => service.uploadFile(
    attachment.file,
    onProgress: onProgress,
    cancellationToken: cancellationToken,
  ),
};

class _AttachmentTrigger extends StatelessWidget {
  final _AttachmentSurface surface;
  final bool formattingOpen;
  final ValueChanged<BuildContext> onTap;

  const _AttachmentTrigger({
    required this.surface,
    required this.formattingOpen,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 240);

    return SizedBox.square(
      dimension: 36,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: context.colors.surface,
          shape: BoxShape.circle,
          border: Border.all(
            color: Colors.black.withValues(alpha: 0.04),
            width: 1,
          ),
        ),
        child: IconButton(
          tooltip: switch (surface) {
            _AttachmentSurface.closed =>
              formattingOpen ? 'Close formatting' : 'Add attachment',
            _AttachmentSurface.menu => 'Close attachments',
            _AttachmentSurface.camera ||
            _AttachmentSurface.photos => 'Back to attachment options',
          },
          onPressed: () => _runComposerAction(() => onTap(context)),
          padding: EdgeInsets.zero,
          visualDensity: VisualDensity.compact,
          icon: AnimatedRotation(
            duration: duration,
            curve: Curves.easeOutBack,
            turns: surface == _AttachmentSurface.menu || formattingOpen
                ? 0.125
                : 0,
            child: AnimatedSwitcher(
              duration: duration,
              switchInCurve: Curves.easeOutBack,
              switchOutCurve: Curves.easeInOutCubic,
              transitionBuilder: (child, animation) => FadeTransition(
                opacity: animation,
                child: ScaleTransition(
                  scale: Tween<double>(begin: 0.92, end: 1).animate(animation),
                  child: child,
                ),
              ),
              child: Icon(
                switch (surface) {
                  _AttachmentSurface.camera => LucideIcons.camera,
                  _AttachmentSurface.photos => LucideIcons.images,
                  _AttachmentSurface.closed ||
                  _AttachmentSurface.menu => LucideIcons.plus,
                },
                key: ValueKey('attachment-trigger-${surface.name}'),
                size: 20,
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _AttachmentMenu extends StatelessWidget {
  final _AttachmentMenuLayout layout;
  final VoidCallback onCamera;
  final VoidCallback onPhotos;
  final VoidCallback onVideo;
  final VoidCallback onVoiceNote;
  final VoidCallback onFiles;

  const _AttachmentMenu({
    required this.layout,
    required this.onCamera,
    required this.onPhotos,
    required this.onVideo,
    required this.onVoiceNote,
    required this.onFiles,
  });

  @override
  Widget build(BuildContext context) {
    final items = <(IconData, String, VoidCallback)>[
      (LucideIcons.camera, 'Camera', onCamera),
      (LucideIcons.images, 'Photos', onPhotos),
      (LucideIcons.video, 'Video', onVideo),
      (LucideIcons.mic, 'Voice note', onVoiceNote),
      (LucideIcons.file, 'Files', onFiles),
    ];
    return SizedBox(
      key: const ValueKey('attachment-menu'),
      width: _attachmentMenuWidth,
      height: layout.height,
      child: ListView.separated(
        key: const ValueKey('attachment-menu-scroll'),
        padding: const EdgeInsets.all(_attachmentMenuPadding),
        physics: layout.isScrollable
            ? null
            : const NeverScrollableScrollPhysics(),
        itemCount: items.length,
        separatorBuilder: (_, _) =>
            const SizedBox(height: _attachmentMenuItemSpacing),
        itemBuilder: (context, index) {
          final (icon, label, onTap) = items[index];
          return _AttachmentMenuItem(
            height: layout.itemHeight,
            icon: icon,
            label: label,
            onTap: onTap,
          );
        },
      ),
    );
  }
}

class _AttachmentMenuItem extends StatelessWidget {
  final double height;
  final IconData icon;
  final String label;
  final VoidCallback onTap;

  const _AttachmentMenuItem({
    required this.height,
    required this.icon,
    required this.label,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      key: ValueKey('attachment-menu-item-${label.toLowerCase()}'),
      height: height,
      child: Tooltip(
        message: label,
        child: InkWell(
          onTap: () => _runComposerAction(onTap),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
            child: Row(
              children: [
                SizedBox(
                  key: ValueKey('attachment-menu-icon-${label.toLowerCase()}'),
                  width: _attachmentMenuIconSlotWidth,
                  child: Center(
                    child: Icon(
                      icon,
                      size: _attachmentMenuIconSize,
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
                const SizedBox(width: Grid.xxs),
                Expanded(
                  child: Text(
                    label,
                    key: ValueKey(
                      'attachment-menu-label-${label.toLowerCase()}',
                    ),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.titleMedium?.copyWith(
                      color: context.colors.onSurface,
                      fontWeight: FontWeight.w400,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

List<_PendingAttachment> _withoutAttachment(
  List<_PendingAttachment> attachments,
  int id,
) {
  return [
    for (final attachment in attachments)
      if (attachment.id != id) attachment,
  ];
}

class _AttachmentStrip extends StatelessWidget {
  final List<_PendingAttachment> attachments;
  final ValueChanged<int> onRemove;

  const _AttachmentStrip({required this.attachments, required this.onRemove});

  @override
  Widget build(BuildContext context) {
    final thumbWidth = 72.0;
    final thumbHeight = 72.0;

    return SizedBox(
      height: thumbHeight,
      child: LayoutBuilder(
        builder: (context, constraints) => ListView.separated(
          scrollDirection: Axis.horizontal,
          itemCount: attachments.length,
          separatorBuilder: (_, _) => const SizedBox(width: Grid.half),
          itemBuilder: (context, index) {
            final attachment = attachments[index];
            if (attachment.kind == _PendingAttachmentKind.voiceNote) {
              return SizedBox(
                width: constraints.maxWidth,
                child: VoiceNoteAttachment.local(
                  path: attachment.file.path,
                  duration: attachment.duration ?? Duration.zero,
                  waveform: attachment.waveform,
                  onRemove: () =>
                      _runComposerAction(() => onRemove(attachment.id)),
                ),
              );
            }
            return Container(
              key: ValueKey('compose-attachment:${attachment.id}'),
              width: thumbWidth,
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(Radii.md),
                border: Border.all(color: context.colors.outlineVariant),
              ),
              child: Stack(
                fit: StackFit.expand,
                children: [
                  ClipRRect(
                    borderRadius: BorderRadius.circular(Radii.md),
                    child: attachment.kind == _PendingAttachmentKind.video
                        ? ColoredBox(
                            color: Colors.black,
                            child: Center(
                              child: Icon(
                                LucideIcons.video,
                                color: Colors.white,
                                size: 24,
                              ),
                            ),
                          )
                        : attachment.kind == _PendingAttachmentKind.image &&
                              attachment.file.path.isNotEmpty
                        ? Image.file(
                            File(attachment.file.path),
                            fit: BoxFit.cover,
                            errorBuilder: (_, _, _) => ColoredBox(
                              color: context.colors.surface,
                              child: Icon(
                                LucideIcons.image,
                                color: context.colors.onSurfaceVariant,
                              ),
                            ),
                          )
                        : attachment.kind == _PendingAttachmentKind.image
                        ? _MemoryAttachmentImage(file: attachment.file)
                        : ColoredBox(
                            color: context.colors.surface,
                            child: Padding(
                              padding: const EdgeInsets.all(Grid.xxs),
                              child: Column(
                                mainAxisAlignment: MainAxisAlignment.center,
                                children: [
                                  Icon(
                                    LucideIcons.file,
                                    color: context.colors.onSurfaceVariant,
                                  ),
                                  const SizedBox(height: Grid.quarter),
                                  Text(
                                    attachment.file.name.isEmpty
                                        ? 'File'
                                        : attachment.file.name,
                                    maxLines: 2,
                                    textAlign: TextAlign.center,
                                    overflow: TextOverflow.ellipsis,
                                    style: context.textTheme.labelSmall
                                        ?.copyWith(
                                          color:
                                              context.colors.onSurfaceVariant,
                                        ),
                                  ),
                                ],
                              ),
                            ),
                          ),
                  ),
                  Positioned(
                    top: Grid.quarter,
                    right: Grid.quarter,
                    child: SizedBox(
                      width: 24,
                      height: 24,
                      child: IconButton(
                        onPressed: () =>
                            _runComposerAction(() => onRemove(attachment.id)),
                        tooltip: 'Remove attachment',
                        visualDensity: VisualDensity.compact,
                        style: IconButton.styleFrom(
                          backgroundColor: context.colors.surface.withValues(
                            alpha: 0.92,
                          ),
                          minimumSize: const Size(24, 24),
                          maximumSize: const Size(24, 24),
                          padding: EdgeInsets.zero,
                          tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                        ),
                        icon: Icon(
                          LucideIcons.x,
                          size: 14,
                          color: context.colors.onSurface,
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            );
          },
        ),
      ),
    );
  }
}

class _MemoryAttachmentImage extends HookConsumerWidget {
  final XFile file;

  const _MemoryAttachmentImage({required this.file});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final bytes = useFuture(useMemoized(() => file.readAsBytes(), [file]));
    final data = bytes.data;

    if (data == null || data.isEmpty) {
      return ColoredBox(
        color: context.colors.surface,
        child: Icon(LucideIcons.image, color: context.colors.onSurfaceVariant),
      );
    }

    return Image.memory(
      data,
      fit: BoxFit.cover,
      errorBuilder: (_, _, _) => ColoredBox(
        color: context.colors.surface,
        child: Icon(LucideIcons.image, color: context.colors.onSurfaceVariant),
      ),
    );
  }
}
