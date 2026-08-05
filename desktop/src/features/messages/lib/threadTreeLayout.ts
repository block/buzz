import { getAvatarScale, getAvatarSizeRem } from "@/shared/lib/avatarScale";

const THREAD_REPLY_MAX_VISIBLE_DEPTH = 6;

export const THREAD_REPLY_ROW_MARGIN_INLINE_REM = 0.25; // Tailwind mx-1
const THREAD_REPLY_ROW_CONTENT_INSET_REM = 0.5; // Tailwind px-2
const THREAD_REPLY_ROW_CONTENT_GAP_REM = 0.625; // Tailwind gap-2.5
export const THREAD_REPLY_ROW_PADDING_TOP_REM = 0.25; // Tailwind py-1
const THREAD_REPLY_AVATAR_LINE_GAP_REM = 0.25; // Tailwind spacing-1

/** Base (unscaled) md avatar size. Prefer {@link getThreadReplyAvatarSizeRem}. */
const THREAD_REPLY_AVATAR_SIZE_BASE_REM = 3;

export function getThreadReplyAvatarSizeRem(scale = getAvatarScale()) {
  return getAvatarSizeRem("md", scale);
}

function getThreadReplyDepthStepRem(scale = getAvatarScale()) {
  // Keep depth indent in step with avatar size so rails stay centered.
  return getThreadReplyAvatarSizeRem(scale);
}

/** @deprecated Prefer {@link getThreadReplyDepthStepRem} / getThreadReplyIndentRem. */
export function getThreadDepthIndentRem(scale = getAvatarScale()) {
  return getThreadReplyDepthStepRem(scale);
}

export function getThreadReplyBodyOffsetRem(scale = getAvatarScale()) {
  return (
    THREAD_REPLY_ROW_MARGIN_INLINE_REM +
    THREAD_REPLY_ROW_CONTENT_INSET_REM +
    getThreadReplyAvatarSizeRem(scale) +
    THREAD_REPLY_ROW_CONTENT_GAP_REM
  );
}

/** @deprecated Prefer {@link getThreadReplyBodyOffsetRem} — value tracks avatar scale. */
export const THREAD_REPLY_BODY_OFFSET_REM =
  THREAD_REPLY_ROW_MARGIN_INLINE_REM +
  THREAD_REPLY_ROW_CONTENT_INSET_REM +
  THREAD_REPLY_AVATAR_SIZE_BASE_REM +
  THREAD_REPLY_ROW_CONTENT_GAP_REM;

export const THREAD_REPLY_ROOT_INDENT_REM = THREAD_REPLY_AVATAR_SIZE_BASE_REM;
export const THREAD_REPLY_NESTED_INDENT_REM = THREAD_REPLY_ROOT_INDENT_REM;
export const THREAD_REPLY_LINE_WIDTH_REM = 0.09375;

function getThreadReplyAvatarCenterOffsetRem(scale = getAvatarScale()) {
  return (
    THREAD_REPLY_ROW_MARGIN_INLINE_REM +
    THREAD_REPLY_ROW_CONTENT_INSET_REM +
    getThreadReplyAvatarSizeRem(scale) / 2
  );
}

function getThreadReplyAvatarCenterYRemValue(scale = getAvatarScale()) {
  return (
    THREAD_REPLY_ROW_PADDING_TOP_REM + getThreadReplyAvatarSizeRem(scale) / 2
  );
}

/** Rail center Y at the given avatar scale (top padding + half avatar). */
export function getThreadRailCenterRem(scale = getAvatarScale()) {
  return getThreadReplyAvatarCenterYRemValue(scale);
}

export function threadReplyLength(valueRem: number) {
  if (valueRem === 0) return "0";
  return `${Number(valueRem.toFixed(5))}rem`;
}

function clampVisibleDepth(depth: number) {
  return Math.min(Math.max(depth, 0), THREAD_REPLY_MAX_VISIBLE_DEPTH);
}

function getThreadReplyVisualDepth(depth: number) {
  return clampVisibleDepth(Math.max(0, depth - 1));
}

function getThreadReplyIndentForVisibleDepthRem(
  visibleDepth: number,
  scale = getAvatarScale(),
) {
  const step = getThreadReplyDepthStepRem(scale);
  return visibleDepth > 0 ? step + (visibleDepth - 1) * step : 0;
}

export function getThreadReplyIndentRem(
  depth: number,
  scale = getAvatarScale(),
) {
  return getThreadReplyIndentForVisibleDepthRem(
    getThreadReplyVisualDepth(depth),
    scale,
  );
}

export function getThreadReplyAvatarCenterRem(
  depth: number,
  scale = getAvatarScale(),
) {
  return (
    getThreadReplyIndentRem(depth, scale) +
    getThreadReplyAvatarCenterOffsetRem(scale)
  );
}

function getThreadReplyAvatarCenterForVisibleDepthRem(
  visibleDepth: number,
  scale = getAvatarScale(),
) {
  return (
    getThreadReplyIndentForVisibleDepthRem(visibleDepth, scale) +
    getThreadReplyAvatarCenterOffsetRem(scale)
  );
}

export function getThreadReplyAvatarCenterYRem(scale = getAvatarScale()) {
  return getThreadReplyAvatarCenterYRemValue(scale);
}

export function getThreadReplyDescendantRailStartYRem(
  scale = getAvatarScale(),
) {
  const avatarSize = getThreadReplyAvatarSizeRem(scale);
  return (
    getThreadReplyAvatarCenterYRemValue(scale) +
    avatarSize / 2 +
    THREAD_REPLY_AVATAR_LINE_GAP_REM
  );
}

export function getThreadReplyConnectorLayout(
  depth: number,
  scale = getAvatarScale(),
) {
  const visibleDepth = getThreadReplyVisualDepth(depth);
  if (visibleDepth === 0) {
    return null;
  }

  const avatarSize = getThreadReplyAvatarSizeRem(scale);
  const avatarRadius = avatarSize / 2;
  const parentOffsetRem = getThreadReplyAvatarCenterForVisibleDepthRem(
    visibleDepth - 1,
    scale,
  );
  const childOffsetRem = getThreadReplyAvatarCenterForVisibleDepthRem(
    visibleDepth,
    scale,
  );
  const childEdgeOffsetRem =
    childOffsetRem - avatarRadius - THREAD_REPLY_AVATAR_LINE_GAP_REM;

  return {
    childOffsetRem,
    heightRem: getThreadReplyAvatarCenterYRemValue(scale),
    parentOffsetRem,
    widthRem: Math.max(0, childEdgeOffsetRem - parentOffsetRem),
  };
}
