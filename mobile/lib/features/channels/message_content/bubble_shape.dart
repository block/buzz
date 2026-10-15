part of '../message_content.dart';

/// Uniform, tail-free corners for every message in a sender run.
class _MessageBubbleShape extends RoundedRectangleBorder {
  const _MessageBubbleShape()
    : super(
        borderRadius: const BorderRadius.all(Radius.circular(Radii.container)),
      );
}

BorderRadius _attachmentCorners({required bool start, required bool end}) =>
    conversationAttachmentCorners;
