import { Check, ChevronDown, Copy, Eye, EyeOff, Pencil } from "lucide-react";
import {
  AnimatePresence,
  LayoutGroup,
  motion,
  useReducedMotion,
} from "motion/react";
import * as React from "react";
import { toast } from "sonner";

import {
  useProfileQuery,
  useUpdateProfileMutation,
} from "@/features/profile/hooks";
import { NsecMaskedDisplay } from "@/features/onboarding/ui/NsecMaskedDisplay";
import { getNsec, signOut } from "@/shared/api/tauriIdentity";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { MaskedAvatarBadgeFrame } from "@/features/profile/ui/MaskedAvatarBadgeFrame";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  ProfileAvatarEditor,
  parseEmojiAvatarDataUrl,
} from "@/features/profile/ui/ProfileAvatarEditor";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import { Textarea } from "@/shared/ui/textarea";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

type ProfileSettingsCardProps = {
  currentPubkey?: string;
  fallbackDisplayName?: string;
};

const AVATAR_EDITOR_TRANSITION_MS = 240;
const AVATAR_PREVIEW_CAPTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;
const AVATAR_MODE_TABS_TRANSITION = {
  duration: 0.2,
  ease: [0.23, 1, 0.32, 1],
} as const;
const AVATAR_EDITOR_LAYOUT_TRANSITION = {
  duration: 0.3,
  ease: [0.23, 1, 0.32, 1],
} as const;

function IdentityRow({
  label,
  value,
  testId,
  copyValue,
}: {
  label: string;
  value: string;
  testId: string;
  copyValue?: string;
}) {
  return (
    <div className="flex min-h-16 items-center justify-between gap-4 px-4 py-3">
      <div className="min-w-0 space-y-1">
        <p className="text-sm font-medium">{label}</p>
        <p
          className="min-w-0 truncate text-sm text-muted-foreground"
          data-testid={testId}
          title={value}
        >
          {value}
        </p>
      </div>
      {copyValue ? (
        <button
          aria-label={`Copy ${label}`}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-muted px-3 py-1.5 text-sm font-medium text-foreground transition-colors hover:bg-muted/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
          data-testid={`copy-${testId}`}
          onClick={async () => {
            await writeTextToClipboard(copyValue);
            toast.success("Copied to clipboard");
          }}
          title={`Copy ${label}`}
          type="button"
        >
          <Copy className="h-4 w-4 shrink-0" />
          Copy
        </button>
      ) : null}
    </div>
  );
}

/**
 * Collapsible row that reveals the user's nsec on demand.
 * The nsec is fetched only when first expanded and cleared on collapse.
 */
function NsecRevealRow() {
  const [isOpen, setIsOpen] = React.useState(false);
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  // Guards against a late-resolving getNsec() repopulating state after Hide
  // or after the settings panel unmounts.
  const fetchCancelledRef = React.useRef(false);

  React.useEffect(() => {
    return () => {
      fetchCancelledRef.current = true;
      setNsec(null);
    };
  }, []);

  async function handleReveal() {
    if (!isOpen) {
      fetchCancelledRef.current = false;
      setIsOpen(true);
      setIsLoading(true);
      setLoadError(null);
      try {
        const value = await getNsec();
        if (!fetchCancelledRef.current) setNsec(value);
      } catch (err) {
        if (!fetchCancelledRef.current)
          setLoadError(
            err instanceof Error
              ? err.message
              : "Failed to retrieve private key.",
          );
      } finally {
        if (!fetchCancelledRef.current) setIsLoading(false);
      }
    } else {
      // Cancel any in-flight fetch before clearing state.
      fetchCancelledRef.current = true;
      setNsec(null);
      setIsOpen(false);
    }
  }

  return (
    <div className="px-4 py-3" data-testid="profile-private-key-row">
      <div className="flex items-center justify-between gap-4">
        <p className="text-sm font-medium">Private key</p>
        <button
          aria-label={isOpen ? "Hide private key" : "Reveal private key"}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-muted px-3 py-1.5 text-sm font-medium text-foreground transition-colors hover:bg-muted/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
          data-testid="profile-private-key-toggle"
          onClick={() => void handleReveal()}
          type="button"
        >
          {isOpen ? (
            <>
              <EyeOff className="h-4 w-4 shrink-0" />
              Hide
            </>
          ) : (
            <>
              <Eye className="h-4 w-4 shrink-0" />
              Reveal
            </>
          )}
        </button>
      </div>
      {isOpen ? (
        <div className="mt-2">
          {isLoading ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : loadError ? (
            <p className="text-sm text-destructive">{loadError}</p>
          ) : nsec ? (
            <NsecMaskedDisplay nsec={nsec} />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function EditProfileMetadataButton({
  label,
  testId,
  onClick,
  disabled,
  isEditing,
}: {
  label: string;
  testId: string;
  onClick: () => void;
  disabled: boolean;
  isEditing: boolean;
}) {
  const Icon = isEditing ? Check : Pencil;
  const actionLabel = isEditing ? "Done" : "Edit";
  const accessibleLabel = isEditing ? `Done editing ${label}` : `Edit ${label}`;

  return (
    <button
      aria-label={accessibleLabel}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-1.5 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60",
        isEditing
          ? "border-transparent bg-primary text-primary-foreground shadow hover:bg-primary/90"
          : "border-transparent bg-muted text-foreground hover:bg-muted/80",
      )}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      title={accessibleLabel}
      type="button"
    >
      <Icon className="h-4 w-4 shrink-0" />
      {actionLabel}
    </button>
  );
}

export function ProfileSettingsCard({
  currentPubkey,
  fallbackDisplayName,
}: ProfileSettingsCardProps) {
  const shouldReduceMotion = useReducedMotion();
  const profileQuery = useProfileQuery();
  const updateProfileMutation = useUpdateProfileMutation();
  const profile = profileQuery.data;

  const currentDisplayName = profile?.displayName ?? "";
  const currentAvatarUrl = profile?.avatarUrl ?? "";
  const currentAbout = profile?.about ?? "";
  const [displayNameDraft, setDisplayNameDraft] = React.useState("");
  const [avatarUrlDraft, setAvatarUrlDraft] = React.useState("");
  const [aboutDraft, setAboutDraft] = React.useState("");
  const [uploadedAvatarUrlDraft, setUploadedAvatarUrlDraft] = React.useState<
    string | null
  >(null);
  const [isAvatarEditorOpen, setIsAvatarEditorOpen] = React.useState(false);
  const [isUploadingAvatar, setIsUploadingAvatar] = React.useState(false);
  const [isAvatarEditorFinishing, setIsAvatarEditorFinishing] =
    React.useState(false);
  // The animated avatar tab portals its camera feed / composed preview into
  // the main avatar preview above, replacing the regular preview while live.
  const [animatedPreviewEl, setAnimatedPreviewEl] =
    React.useState<HTMLDivElement | null>(null);
  const [avatarModeTabsEl, setAvatarModeTabsEl] =
    React.useState<HTMLDivElement | null>(null);
  const [isAnimatedPreviewActive, setIsAnimatedPreviewActive] =
    React.useState(false);
  const [animatedPreviewCaption, setAnimatedPreviewCaption] = React.useState<
    string | null
  >(null);
  const [isEditingProfileMetadata, setIsEditingProfileMetadata] =
    React.useState(false);
  const [shouldRenderAvatarEditor, setShouldRenderAvatarEditor] =
    React.useState(false);
  const [avatarSquishKey, setAvatarSquishKey] = React.useState(0);
  const [isSignOutOpen, setIsSignOutOpen] = React.useState(false);
  const [isSignOutPending, setIsSignOutPending] = React.useState(false);
  const displayNameInputRef = React.useRef<HTMLInputElement>(null);
  const aboutTextareaRef = React.useRef<HTMLTextAreaElement>(null);
  const sectionRef = React.useRef<HTMLElement>(null);
  const isEditingProfileMetadataRef = React.useRef(false);
  const avatarEditorOpenFrameRef = React.useRef<number | null>(null);
  const avatarEditorFinishTimeoutRef = React.useRef<number | null>(null);
  const savedScrollTopRef = React.useRef<number | null>(null);
  isEditingProfileMetadataRef.current = isEditingProfileMetadata;

  React.useEffect(() => {
    if (!isEditingProfileMetadataRef.current) {
      setDisplayNameDraft(currentDisplayName);
    }
  }, [currentDisplayName]);

  React.useEffect(() => {
    if (!isAvatarEditorOpen) {
      setAvatarUrlDraft(currentAvatarUrl);
    }
  }, [currentAvatarUrl, isAvatarEditorOpen]);

  React.useEffect(() => {
    if (!isEditingProfileMetadataRef.current) {
      setAboutDraft(currentAbout);
    }
  }, [currentAbout]);

  React.useEffect(() => {
    if (
      uploadedAvatarUrlDraft &&
      currentAvatarUrl &&
      uploadedAvatarUrlDraft !== currentAvatarUrl &&
      avatarUrlDraft !== uploadedAvatarUrlDraft
    ) {
      setUploadedAvatarUrlDraft(null);
    }
  }, [avatarUrlDraft, currentAvatarUrl, uploadedAvatarUrlDraft]);

  React.useEffect(() => {
    if (isEditingProfileMetadata) {
      displayNameInputRef.current?.focus();
    }
  }, [isEditingProfileMetadata]);

  React.useEffect(() => {
    if (
      isAvatarEditorOpen ||
      !shouldRenderAvatarEditor ||
      isAvatarEditorFinishing
    ) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setShouldRenderAvatarEditor(false);
    }, AVATAR_EDITOR_TRANSITION_MS);

    return () => window.clearTimeout(timeoutId);
  }, [isAvatarEditorFinishing, isAvatarEditorOpen, shouldRenderAvatarEditor]);

  React.useEffect(() => {
    if (!shouldRenderAvatarEditor) {
      setIsAvatarEditorFinishing(false);
    }
  }, [shouldRenderAvatarEditor]);

  React.useEffect(() => {
    return () => {
      if (avatarEditorOpenFrameRef.current !== null) {
        window.cancelAnimationFrame(avatarEditorOpenFrameRef.current);
      }
      if (avatarEditorFinishTimeoutRef.current !== null) {
        window.clearTimeout(avatarEditorFinishTimeoutRef.current);
      }
    };
  }, []);

  const nextDisplayName = displayNameDraft.trim();
  const nextAvatarUrl = avatarUrlDraft.trim();
  const nextAbout = aboutDraft.trim();
  const updatePayload = React.useMemo(() => {
    const payload: {
      displayName?: string;
      avatarUrl?: string;
      about?: string;
    } = {};

    if (nextDisplayName.length > 0 && nextDisplayName !== currentDisplayName) {
      payload.displayName = nextDisplayName;
    }
    if (nextAvatarUrl.length > 0 && nextAvatarUrl !== currentAvatarUrl) {
      payload.avatarUrl = nextAvatarUrl;
    }
    if (nextAbout !== currentAbout) {
      payload.about = nextAbout;
    }

    return payload;
  }, [
    currentAbout,
    currentAvatarUrl,
    currentDisplayName,
    nextAbout,
    nextAvatarUrl,
    nextDisplayName,
  ]);

  const hasPendingDisplayNameClearRequest =
    currentDisplayName.length > 0 && nextDisplayName.length === 0;
  const hasPendingAvatarClearRequest =
    currentAvatarUrl.length > 0 && nextAvatarUrl.length === 0;
  const hasPendingClearRequest =
    hasPendingDisplayNameClearRequest || hasPendingAvatarClearRequest;
  const hasProfileChanges = Object.keys(updatePayload).length > 0;
  const canSave =
    hasProfileChanges && !updateProfileMutation.isPending && !isUploadingAvatar;
  const isAvatarEditorSaving =
    isAvatarEditorFinishing ||
    (shouldRenderAvatarEditor && updateProfileMutation.isPending);
  const shouldShowSaveArea = hasPendingClearRequest;
  const readOnlyContentMotionClassName = cn(
    "min-w-0 w-full origin-top overflow-hidden transition-[opacity,scale] duration-200 ease-out will-change-[opacity,transform]",
    shouldRenderAvatarEditor ? "absolute inset-x-0 top-0" : "relative",
    isAvatarEditorOpen
      ? "pointer-events-none scale-[0.98] opacity-0"
      : "scale-100 opacity-100",
  );

  const resolvedName =
    nextDisplayName ||
    profile?.displayName ||
    fallbackDisplayName ||
    "Your profile";
  const resolvedPubkey = profile?.pubkey ?? currentPubkey ?? "Unavailable";
  const nip05Handle = profile?.nip05Handle ?? "Not set";
  const emojiAvatarPreview = React.useMemo(
    () => parseEmojiAvatarDataUrl(avatarUrlDraft),
    [avatarUrlDraft],
  );
  const shouldShowAnimatedPreview =
    isAvatarEditorOpen && isAnimatedPreviewActive;
  const visibleAnimatedPreviewCaption = isAvatarEditorOpen
    ? animatedPreviewCaption
    : null;
  const avatarEditorLayoutTransition = shouldReduceMotion
    ? { duration: 0 }
    : AVATAR_EDITOR_LAYOUT_TRANSITION;
  const avatarEditShellClassName = cn(
    "flex h-[54px] w-[54px] items-center justify-center rounded-full opacity-100 transition-[opacity,scale,transform] duration-150 ease-out",
    isAvatarEditorOpen
      ? "pointer-events-none scale-[0.94] opacity-0"
      : "scale-100 opacity-100",
  );
  const avatarEditButtonClassName = cn(
    "flex h-11 w-11 items-center justify-center rounded-full bg-sidebar-active text-sidebar-active-foreground transition-[background-color,opacity,scale,transform] duration-150 ease-out hover:scale-[1.04] hover:bg-sidebar-active focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-90 disabled:hover:scale-100",
  );
  const clearAvatarEditorFinishTimeout = React.useCallback(() => {
    if (avatarEditorFinishTimeoutRef.current === null) {
      return;
    }
    window.clearTimeout(avatarEditorFinishTimeoutRef.current);
    avatarEditorFinishTimeoutRef.current = null;
  }, []);
  const saveScrollPosition = React.useCallback(() => {
    const el = sectionRef.current;
    if (!el) return;
    const scroller = el.closest<HTMLElement>("[class*='overflow-y']");
    if (scroller) savedScrollTopRef.current = scroller.scrollTop;
  }, []);
  const restoreScrollPosition = React.useCallback(() => {
    const saved = savedScrollTopRef.current;
    if (saved == null) return;
    savedScrollTopRef.current = null;
    const el = sectionRef.current;
    if (!el) return;
    const scroller = el.closest<HTMLElement>("[class*='overflow-y']");
    if (scroller) scroller.scrollTop = saved;
  }, []);
  const closeAvatarEditor = React.useCallback(() => {
    clearAvatarEditorFinishTimeout();
    setIsAvatarEditorOpen(false);
    setIsAvatarEditorFinishing(false);
    restoreScrollPosition();
  }, [clearAvatarEditorFinishTimeout, restoreScrollPosition]);
  const completeAvatarEditorClose = React.useCallback(() => {
    setIsAvatarEditorOpen(false);
    clearAvatarEditorFinishTimeout();
    restoreScrollPosition();
    avatarEditorFinishTimeoutRef.current = window.setTimeout(
      () => {
        avatarEditorFinishTimeoutRef.current = null;
        setIsAvatarEditorFinishing(false);
      },
      shouldReduceMotion ? 0 : AVATAR_EDITOR_TRANSITION_MS,
    );
  }, [
    clearAvatarEditorFinishTimeout,
    restoreScrollPosition,
    shouldReduceMotion,
  ]);
  const reopenAvatarEditorAfterClose = React.useCallback(() => {
    clearAvatarEditorFinishTimeout();
    setShouldRenderAvatarEditor(true);
    setIsAvatarEditorFinishing(false);
    setIsAvatarEditorOpen(true);
  }, [clearAvatarEditorFinishTimeout]);

  const openAvatarEditor = React.useCallback(() => {
    saveScrollPosition();
    setShouldRenderAvatarEditor(true);
    setIsAvatarEditorFinishing(false);
    clearAvatarEditorFinishTimeout();

    if (avatarEditorOpenFrameRef.current !== null) {
      window.cancelAnimationFrame(avatarEditorOpenFrameRef.current);
    }

    avatarEditorOpenFrameRef.current = window.requestAnimationFrame(() => {
      avatarEditorOpenFrameRef.current = null;
      setIsAvatarEditorOpen(true);
    });
  }, [clearAvatarEditorFinishTimeout, saveScrollPosition]);

  const saveProfile = React.useCallback(async () => {
    if (!canSave) {
      return false;
    }

    await updateProfileMutation.mutateAsync(updatePayload);
    setIsEditingProfileMetadata(false);
    setDisplayNameDraft(updatePayload.displayName ?? currentDisplayName);
    setAvatarUrlDraft(updatePayload.avatarUrl ?? currentAvatarUrl);
    setAboutDraft(updatePayload.about ?? currentAbout);
    toast.success("Profile saved");
    return true;
  }, [
    canSave,
    currentAbout,
    currentAvatarUrl,
    currentDisplayName,
    updatePayload,
    updateProfileMutation,
  ]);

  const handleProfileMetadataEdit = React.useCallback(() => {
    if (!isEditingProfileMetadata) {
      setIsEditingProfileMetadata(true);
      return;
    }

    if (!hasProfileChanges) {
      if (hasPendingDisplayNameClearRequest) {
        setDisplayNameDraft(currentDisplayName);
      }
      if (hasPendingAvatarClearRequest) {
        setAvatarUrlDraft(currentAvatarUrl);
      }
      setIsEditingProfileMetadata(false);
      return;
    }

    void saveProfile();
  }, [
    currentAvatarUrl,
    currentDisplayName,
    hasPendingAvatarClearRequest,
    hasPendingDisplayNameClearRequest,
    hasProfileChanges,
    isEditingProfileMetadata,
    saveProfile,
  ]);

  const handleAvatarEditorDone = React.useCallback(() => {
    if (!hasProfileChanges) {
      if (hasPendingAvatarClearRequest) {
        setAvatarUrlDraft(currentAvatarUrl);
      }
      closeAvatarEditor();
      return;
    }

    setIsAvatarEditorFinishing(true);
    void saveProfile()
      .then((didSave) => {
        if (didSave) {
          completeAvatarEditorClose();
          return;
        }

        reopenAvatarEditorAfterClose();
      })
      .catch(() => {
        reopenAvatarEditorAfterClose();
      });
  }, [
    closeAvatarEditor,
    completeAvatarEditorClose,
    currentAvatarUrl,
    hasPendingAvatarClearRequest,
    hasProfileChanges,
    reopenAvatarEditorAfterClose,
    saveProfile,
  ]);

  const animateEmojiAvatarChange = React.useCallback(() => {
    setAvatarSquishKey((key) => key + 1);
  }, []);

  return (
    <section
      className="min-w-0"
      data-testid="settings-profile"
      ref={sectionRef}
    >
      <div>
        <SettingsSectionHeader
          title="Profile"
          description="Update how your name, avatar, and bio appear across Buzz."
        />

        <div className="space-y-3">
          {profileQuery.error instanceof Error ? (
            <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {profileQuery.error.message}
            </p>
          ) : null}

          {updateProfileMutation.error instanceof Error ? (
            <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {updateProfileMutation.error.message}
            </p>
          ) : null}

          <div className="min-w-0">
            <form
              className="min-w-0 space-y-3"
              id="profile-settings-form"
              onSubmit={(event) => {
                event.preventDefault();
                void saveProfile();
              }}
            >
              <LayoutGroup id="profile-avatar-editor-layout">
                <motion.div
                  className="flex min-w-0 flex-col items-center gap-12"
                  layout="position"
                  transition={avatarEditorLayoutTransition}
                >
                  <AnimatePresence initial={false} mode="popLayout">
                    {isAvatarEditorOpen ? (
                      <motion.div
                        animate={{ opacity: 1, scale: 1 }}
                        className="relative z-20 -mb-14 grid h-48 w-full max-w-[576px] origin-center place-items-center"
                        data-testid="profile-avatar-mode-tabs-slot"
                        exit={
                          shouldReduceMotion
                            ? { opacity: 0 }
                            : { opacity: 0, scale: 0.96 }
                        }
                        initial={
                          shouldReduceMotion
                            ? { opacity: 0 }
                            : { opacity: 0, scale: 0.96 }
                        }
                        key="profile-avatar-mode-tabs-slot"
                        layout="position"
                        ref={setAvatarModeTabsEl}
                        transition={AVATAR_MODE_TABS_TRANSITION}
                      />
                    ) : null}
                  </AnimatePresence>

                  <motion.div
                    className="flex flex-col items-center gap-3"
                    layout="position"
                    transition={avatarEditorLayoutTransition}
                  >
                    <div
                      className="relative h-48 w-48"
                      data-testid="profile-avatar-clip-frame"
                    >
                      <MaskedAvatarBadgeFrame
                        badge={
                          isAvatarEditorOpen ? null : (
                            <div
                              className={avatarEditShellClassName}
                              data-testid="profile-avatar-edit-shell"
                            >
                              <button
                                aria-expanded={isAvatarEditorOpen}
                                aria-label={
                                  isAvatarEditorSaving
                                    ? "Saving profile photo"
                                    : "Edit profile photo"
                                }
                                className={avatarEditButtonClassName}
                                data-testid="profile-avatar-edit"
                                disabled={isAvatarEditorSaving}
                                onClick={openAvatarEditor}
                                title={
                                  isAvatarEditorSaving
                                    ? "Saving profile photo"
                                    : "Edit profile photo"
                                }
                                type="button"
                              >
                                {isAvatarEditorSaving && !isAvatarEditorOpen ? (
                                  <Spinner
                                    aria-label="Saving avatar"
                                    className="h-4 w-4 border-2"
                                  />
                                ) : (
                                  <Pencil className="h-4 w-4" />
                                )}
                              </button>
                            </div>
                          )
                        }
                        badgeBox={{
                          bottom: 0,
                          height: 54,
                          right: 0,
                          width: 54,
                        }}
                        className="h-48 w-48"
                        clipTestId="profile-avatar-preview-clip"
                        cutout={{ cx: 165, cy: 165, r: 30 }}
                        size={192}
                      >
                        <div className="relative h-full w-full">
                          <div
                            className="pointer-events-none absolute inset-0 z-10"
                            data-testid="profile-avatar-animated-preview-slot"
                            ref={setAnimatedPreviewEl}
                          />
                          {shouldShowAnimatedPreview ? null : emojiAvatarPreview ? (
                            <div
                              aria-label={`${resolvedName} avatar`}
                              className="relative flex h-full w-full shrink-0 items-center justify-center overflow-hidden rounded-full shadow-xs"
                              data-testid="profile-avatar-preview"
                              role="img"
                              style={{
                                backgroundColor: emojiAvatarPreview.color,
                              }}
                            >
                              <span
                                className={cn(
                                  "buzz-avatar-emoji-glyph flex h-full w-full items-center justify-center text-[6rem] leading-[6.25rem]",
                                  avatarSquishKey > 0 && "buzz-avatar-squish",
                                )}
                                data-testid="profile-avatar-preview-emoji"
                                key={avatarSquishKey}
                              >
                                {emojiAvatarPreview.emoji}
                              </span>
                            </div>
                          ) : (
                            <ProfileAvatar
                              avatarUrl={avatarUrlDraft || null}
                              className="h-full w-full rounded-full text-5xl"
                              iconClassName="h-14 w-14"
                              label={resolvedName}
                              testId="profile-avatar-preview"
                            />
                          )}
                        </div>
                      </MaskedAvatarBadgeFrame>
                    </div>

                    <AnimatePresence initial={false} mode="wait">
                      {visibleAnimatedPreviewCaption ? (
                        <motion.p
                          animate={{ opacity: 1, y: 0 }}
                          className="w-48 text-center text-sm text-muted-foreground"
                          exit={
                            shouldReduceMotion
                              ? { opacity: 0, y: 0 }
                              : { opacity: 0, y: -4 }
                          }
                          initial={
                            shouldReduceMotion
                              ? { opacity: 0, y: 0 }
                              : { opacity: 0, y: 6 }
                          }
                          key={visibleAnimatedPreviewCaption}
                          transition={AVATAR_PREVIEW_CAPTION_TRANSITION}
                        >
                          {visibleAnimatedPreviewCaption}
                        </motion.p>
                      ) : null}
                    </AnimatePresence>
                  </motion.div>

                  <motion.div
                    className="relative min-w-0 w-full"
                    layout="position"
                    transition={avatarEditorLayoutTransition}
                  >
                    <div
                      className={readOnlyContentMotionClassName}
                      data-testid="profile-readonly-content"
                      inert={isAvatarEditorOpen ? true : undefined}
                    >
                      <div className="space-y-12">
                        <div
                          className="overflow-hidden rounded-xl border border-border/70 bg-background/70 shadow-xs divide-y divide-border/55"
                          data-testid="profile-metadata-card"
                        >
                          <div className="flex min-h-14 items-center justify-between gap-4 px-4 py-3">
                            <h2 className="text-lg font-semibold tracking-tight">
                              Profile info
                            </h2>
                            <EditProfileMetadataButton
                              disabled={updateProfileMutation.isPending}
                              isEditing={isEditingProfileMetadata}
                              label="profile info"
                              onClick={handleProfileMetadataEdit}
                              testId="profile-metadata-edit"
                            />
                          </div>

                          <div className="flex min-h-16 items-center gap-4 px-4 py-3">
                            <div className="min-w-0 flex-1 space-y-1">
                              <label
                                className="block text-sm font-medium"
                                htmlFor="profile-display-name"
                              >
                                Display name
                              </label>
                              {isEditingProfileMetadata ? (
                                <Input
                                  className="h-auto border-0 bg-transparent px-0 py-0 text-sm text-muted-foreground shadow-none placeholder:text-muted-foreground/60 focus-visible:ring-0"
                                  data-testid="profile-display-name"
                                  disabled={updateProfileMutation.isPending}
                                  id="profile-display-name"
                                  onChange={(event) =>
                                    setDisplayNameDraft(event.target.value)
                                  }
                                  placeholder="Display name"
                                  ref={displayNameInputRef}
                                  value={displayNameDraft}
                                />
                              ) : (
                                <p
                                  className="min-w-0 truncate text-sm text-muted-foreground"
                                  data-testid="profile-display-name-value"
                                  title={displayNameDraft || "Not set"}
                                >
                                  {displayNameDraft || "Not set"}
                                </p>
                              )}
                            </div>
                          </div>

                          <div className="flex min-h-16 items-center gap-4 px-4 py-3">
                            <div className="min-w-0 flex-1 space-y-1">
                              <label
                                className="block text-sm font-medium"
                                htmlFor="profile-about"
                              >
                                Profile description
                              </label>
                              {isEditingProfileMetadata ? (
                                <Textarea
                                  className="min-h-[72px] resize-none border-0 bg-transparent px-0 py-0 text-sm leading-6 text-muted-foreground shadow-none placeholder:text-muted-foreground/60 focus-visible:ring-0"
                                  data-testid="profile-about"
                                  disabled={updateProfileMutation.isPending}
                                  id="profile-about"
                                  onChange={(event) =>
                                    setAboutDraft(event.target.value)
                                  }
                                  placeholder="Profile description"
                                  ref={aboutTextareaRef}
                                  value={aboutDraft}
                                />
                              ) : (
                                <p
                                  className={cn(
                                    "min-w-0 break-words text-sm",
                                    aboutDraft
                                      ? "text-muted-foreground"
                                      : "text-muted-foreground/55",
                                  )}
                                  data-testid="profile-about-value"
                                  title={aboutDraft || "Not set"}
                                >
                                  {aboutDraft || "Not set"}
                                </p>
                              )}
                            </div>
                          </div>
                        </div>

                        <div>
                          <details
                            className="group overflow-hidden rounded-xl border border-border/70 bg-background/70 shadow-xs"
                            data-testid="profile-identity-card"
                          >
                            <summary
                              className="group/identity flex cursor-pointer list-none items-center justify-between gap-4 px-4 py-3 text-sm transition-colors duration-150 ease-out hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring [&::-webkit-details-marker]:hidden"
                              data-testid="profile-identity-toggle"
                            >
                              <div className="min-w-0">
                                <h2 className="text-lg font-semibold tracking-tight">
                                  Identity
                                </h2>
                                <p className="mt-1 text-sm font-normal text-muted-foreground">
                                  Your keypair and NIP-05 handle are fixed for
                                  this device.
                                </p>
                              </div>
                              <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-[color,transform] duration-150 ease-out group-open:rotate-180 group-hover/identity:text-foreground group-focus-visible/identity:text-foreground" />
                            </summary>
                            <div
                              className="border-t border-border/55 divide-y divide-border/55"
                              data-testid="profile-identity-details"
                            >
                              <IdentityRow
                                copyValue={
                                  profile?.pubkey ?? currentPubkey ?? undefined
                                }
                                label="Public key"
                                testId="profile-pubkey"
                                value={resolvedPubkey}
                              />
                              <IdentityRow
                                copyValue={profile?.nip05Handle ?? undefined}
                                label="NIP-05 handle"
                                testId="profile-nip05"
                                value={nip05Handle}
                              />
                              <NsecRevealRow />
                            </div>
                          </details>
                        </div>
                      </div>
                    </div>

                    {shouldRenderAvatarEditor ? (
                      <div
                        className={cn(
                          "relative origin-top transition-[opacity,scale] duration-200 ease-out will-change-[opacity,transform]",
                          isAvatarEditorOpen
                            ? "scale-100 opacity-100"
                            : "pointer-events-none scale-[0.98] opacity-0",
                          isAvatarEditorFinishing ? "pointer-events-none" : "",
                        )}
                        aria-busy={isAvatarEditorSaving ? true : undefined}
                        data-testid="profile-avatar-editor-shell"
                        inert={isAvatarEditorOpen ? undefined : true}
                      >
                        <ProfileAvatarEditor
                          animatedPreviewContainer={animatedPreviewEl}
                          avatarUrl={avatarUrlDraft}
                          disabled={isAvatarEditorSaving}
                          donePending={isAvatarEditorSaving}
                          modeTabsContainer={avatarModeTabsEl}
                          onAnimatedPreviewActiveChange={
                            setIsAnimatedPreviewActive
                          }
                          onAnimatedPreviewCaptionChange={
                            setAnimatedPreviewCaption
                          }
                          onDone={handleAvatarEditorDone}
                          onEmojiAvatarChange={animateEmojiAvatarChange}
                          onUploadedAvatarChange={setUploadedAvatarUrlDraft}
                          onUploadingChange={setIsUploadingAvatar}
                          onUrlChange={(url) => setAvatarUrlDraft(url)}
                          previewName={resolvedName}
                          testIdPrefix="profile-avatar"
                        />
                      </div>
                    ) : null}
                  </motion.div>
                </motion.div>
              </LayoutGroup>

              {shouldShowSaveArea && !isAvatarEditorOpen ? (
                <div className="mx-auto w-full max-w-[576px] space-y-2">
                  {hasPendingClearRequest ? (
                    <p className="text-sm text-muted-foreground">
                      Clearing existing profile fields is not supported yet.
                      Blank display name and avatar values are ignored for now.
                    </p>
                  ) : null}
                </div>
              ) : null}
            </form>
          </div>
        </div>
      </div>

      <div
        className="mt-8 border-t border-border/60 pb-6 pt-5"
        data-testid="settings-signout"
      >
        <div className="flex items-center justify-between gap-4 px-1">
          <div className="min-w-0 space-y-1">
            <h2 className="text-lg font-semibold tracking-tight">Sign out</h2>
            <p className="text-sm text-muted-foreground">
              Removes your identity key and all local app data from this device.
              Back up your private key (nsec) first — this cannot be undone.
            </p>
          </div>
          <Button
            className="shrink-0"
            data-testid="signout-open-dialog"
            disabled={isSignOutPending}
            onClick={() => setIsSignOutOpen(true)}
            type="button"
            variant="destructive"
          >
            {isSignOutPending ? (
              <Spinner aria-label="Signing out" className="h-4 w-4 border-2" />
            ) : null}
            {isSignOutPending ? "Signing out…" : "Sign Out"}
          </Button>
        </div>
        <AlertDialog
          onOpenChange={(open) => {
            if (!open && !isSignOutPending) setIsSignOutOpen(false);
          }}
          open={isSignOutOpen}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Sign out and wipe all data?</AlertDialogTitle>
              <AlertDialogDescription>
                This will delete your identity key, all agent settings, and
                cached data from this device, then relaunch Buzz into first-run
                setup. Make sure you have your private key (nsec) backed up
                before continuing — this cannot be undone.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel disabled={isSignOutPending}>
                Cancel
              </AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-destructive-foreground shadow-xs hover:bg-destructive/90"
                data-testid="signout-confirm"
                disabled={isSignOutPending}
                onClick={() => {
                  setIsSignOutPending(true);
                  // Keep the pending state if signOut() resolves before restart.
                  signOut().catch((err: unknown) => {
                    setIsSignOutPending(false);
                    setIsSignOutOpen(false);
                    toast.error(
                      err instanceof Error ? err.message : "Sign out failed.",
                    );
                  });
                }}
              >
                {isSignOutPending ? "Signing out…" : "Sign Out"}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </section>
  );
}
