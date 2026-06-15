import * as React from "react";
import ReactMarkdown, {
  type Components,
  defaultUrlTransform,
} from "react-markdown";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { Copy, Download, FileText } from "lucide-react";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { toast } from "sonner";

import {
  getSingletonHighlighter,
  type HighlighterGeneric,
  type BundledLanguage,
  type BundledTheme,
  type ThemedToken,
} from "shiki";

import { useTheme } from "@/shared/theme/ThemeProvider";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  isMessageLink,
  parseMessageLink,
  type ParsedMessageLink,
} from "@/features/messages/lib/messageLink";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { invokeTauri } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { copyCodeBlockToClipboard } from "@/shared/lib/codeBlockClipboard";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import rehypeImageGallery from "@/shared/lib/rehypeImageGallery";
import rehypeSearchHighlight from "@/shared/lib/rehypeSearchHighlight";
import remarkChannelLinks from "@/shared/lib/remarkChannelLinks";
import remarkCustomEmoji, {
  type CustomEmoji,
} from "@/shared/lib/remarkCustomEmoji";
import remarkMentions from "@/shared/lib/remarkMentions";
import remarkMessageLinks from "@/features/messages/lib/remarkMessageLinks";
import { Button } from "@/shared/ui/button";
import {
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
} from "@/shared/ui/mentionChip";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import {
  classifyChildren,
  hasBlockMedia,
  isImageOnlyParagraph,
  shallowArrayEqual,
} from "./markdownUtils";
import { resolveFileCard } from "./markdownFileCard";
import { VideoPlayer, type VideoReviewContext } from "./VideoPlayer";

type ImetaEntry = {
  dim?: string;
  image?: string;
  thumb?: string;
  m?: string;
  size?: number;
  filename?: string;
  duration?: number;
};

type ImetaLookup = Map<string, ImetaEntry>;

let shikiHighlighter: HighlighterGeneric<BundledLanguage, BundledTheme> | null =
  null;
let shikiInitPromise: Promise<void> | null = null;
const loadedLangs = new Set<string>();
const loadedThemes = new Set<string>();
const tokenCache = new Map<string, ThemedToken[][]>();
const MAX_CACHE_ENTRIES = 100;
const MAX_LOADED_LANGUAGES = 30;
const MAX_HIGHLIGHT_LINES = 150;
const CODE_BLOCK_CLASS =
  "code-block-lines block min-w-full whitespace-pre font-mono text-[13px] leading-6 text-foreground";
const DIFF_ADD_RE = /\s*\/\/\s*\[!code\s*\+\+\]\s*$/;
const DIFF_REMOVE_RE = /\s*\/\/\s*\[!code\s*--\]\s*$/;

function ensureHighlighter(): Promise<void> {
  if (shikiHighlighter) return Promise.resolve();
  if (!shikiInitPromise) {
    shikiInitPromise = getSingletonHighlighter({
      themes: [],
      langs: [],
    }).then((h) => {
      shikiHighlighter = h;
    });
  }
  return shikiInitPromise;
}

function extractLanguage(className?: string): string {
  if (typeof className !== "string") return "";
  const match = className.match(/language-(\S+)/);
  return match ? match[1] : "";
}

function stripDiffMarker(tokens: ThemedToken[], marker: RegExp): ThemedToken[] {
  const last = tokens[tokens.length - 1];
  if (!last) return tokens;
  const stripped = last.content.replace(marker, "");
  if (stripped === last.content) return tokens;
  if (stripped === "") return tokens.slice(0, -1);
  return [...tokens.slice(0, -1), { ...last, content: stripped }];
}

function useStableArray<T>(arr: T[]): T[] {
  const ref = React.useRef(arr);
  if (
    arr.length !== ref.current.length ||
    arr.some((item, i) => item !== ref.current[i])
  ) {
    ref.current = arr;
  }
  return ref.current;
}

function aspectRatioFromDim(dim?: string): number | undefined {
  if (!dim) return undefined;
  const match = dim.match(/^(\d+)x(\d+)$/i);
  if (!match) return undefined;
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (!Number.isFinite(width) || !Number.isFinite(height) || height <= 0) {
    return undefined;
  }
  return width / height;
}

/**
 * Video review context flows through React context instead of
 * `createMarkdownComponents` arguments. The component map must keep a stable
 * identity across re-renders: a new map means new element types, which makes
 * React unmount and remount every rendered node — including `<video>`
 * elements, killing playback (and any in-progress review comment draft)
 * whenever the timeline re-renders.
 */
const VideoReviewMarkdownContext = React.createContext<
  VideoReviewContext | undefined
>(undefined);

function MarkdownVideoPlayer({
  alt,
  entry,
  resolvedSrc,
  src,
}: {
  alt?: string;
  entry?: ImetaEntry;
  resolvedSrc: string;
  src?: string;
}) {
  const videoReviewContext = React.useContext(VideoReviewMarkdownContext);
  // Look up poster frame from imeta tags (NIP-71 `image` field).
  // Fall back to `thumb` for compatibility with older events.
  const posterUrl = entry?.image ?? entry?.thumb;
  const resolvedPoster = posterUrl ? rewriteRelayUrl(posterUrl) : undefined;
  const resolvedReviewContext = React.useMemo(
    () =>
      videoReviewContext
        ? {
            ...videoReviewContext,
            title:
              videoReviewContext.title ?? entry?.filename ?? alt ?? "Video",
          }
        : undefined,
    [alt, entry?.filename, videoReviewContext],
  );

  return (
    <VideoPlayer
      src={resolvedSrc}
      aspectRatio={aspectRatioFromDim(entry?.dim)}
      poster={resolvedPoster}
      durationSeconds={entry?.duration}
      reviewKey={src ?? resolvedSrc}
      reviewContext={resolvedReviewContext}
    />
  );
}

/**
 * `urlTransform` for `<ReactMarkdown>` that preserves `buzz://message?…`
 * links. The default transform strips unknown schemes (returns `""`) before
 * the `a` component override can see them, which would break copy → paste →
 * click end-to-end. Everything else delegates to `defaultUrlTransform`.
 */
function messageLinkUrlTransform(value: string, key: string): string {
  if (key === "href" && isMessageLink(value)) {
    return value;
  }
  return defaultUrlTransform(value);
}

type MarkdownProps = {
  channelNames?: string[];
  className?: string;
  compact?: boolean;
  content: string;
  customEmoji?: CustomEmoji[];
  imetaByUrl?: ImetaLookup;
  interactive?: boolean;
  agentMentionPubkeysByName?: Record<string, string>;
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  searchQuery?: string;
  tight?: boolean;
  videoReviewContext?: VideoReviewContext;
};

type MarkdownVariant = "default" | "compact" | "tight";

/**
 * Inline image embed with click-to-zoom lightbox and right-click download.
 *
 * IMPORTANT: this component renders the inline `<img>` with NO wrapping div
 * and drives the Dialog via controlled `open` state — *not* via Radix's
 * `<Trigger asChild>` cloning onto a wrapper div. An earlier version used
 * the Trigger-asChild pattern around a `<div>` around the `<img>`, which
 * caused a 1-2px layout reflow in the surrounding message body on hover
 * (the gap between the username header and the body would visibly grow).
 * The nested wrapper divs + Radix attribute cloning were repainting the
 * surrounding inline flow on `:hover` state changes. Keeping the `<img>`
 * bare and managing the lightbox + context menu via React state avoids it.
 */
function ImageBlock({
  alt,
  resolvedSrc,
  src,
}: {
  alt: string | undefined;
  resolvedSrc: string | undefined;
  src: string | undefined;
}) {
  const [lightboxOpen, setLightboxOpen] = React.useState(false);
  const [menu, setMenu] = React.useState<{ x: number; y: number } | null>(null);

  React.useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    // Defer attaching the dismiss listeners until after the current event
    // loop turn. The right-click that opens the menu (a `contextmenu` on
    // mousedown) is often followed by a trailing `click`/`pointerup` on the
    // same interaction; attaching synchronously lets that trailing event —
    // and the platform `click` some webviews emit on right-button release —
    // immediately dismiss the menu, so it only flashes. Deferring guarantees
    // the opening interaction can never be the one that closes it.
    let attached = false;
    const timer = window.setTimeout(() => {
      attached = true;
      window.addEventListener("click", close);
      window.addEventListener("contextmenu", close);
      window.addEventListener("scroll", close, true);
    }, 0);
    return () => {
      window.clearTimeout(timer);
      if (attached) {
        window.removeEventListener("click", close);
        window.removeEventListener("contextmenu", close);
        window.removeEventListener("scroll", close, true);
      }
    };
  }, [menu]);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    e.nativeEvent.stopImmediatePropagation();
    setMenu({ x: e.clientX, y: e.clientY });
  };

  const handleDownload = () => {
    setMenu(null);
    if (!src) return;
    invokeTauri("download_image", { url: src }).catch((err: unknown) => {
      const msg = err instanceof Error ? err.message : "Download failed";
      toast.error(msg);
    });
  };

  return (
    <>
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: image opens lightbox on click; keyboard equivalent handled by lightbox close button */}
      <img
        alt={alt}
        className="mt-1 block max-h-64 max-w-sm cursor-pointer rounded-xl object-contain"
        src={resolvedSrc}
        onClick={() => setLightboxOpen(true)}
        onContextMenuCapture={handleContextMenu}
      />
      {menu && src ? (
        <div
          className="fixed z-[100] min-w-[160px] rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
          style={{ left: menu.x, top: menu.y }}
        >
          <button
            type="button"
            className="flex w-full cursor-default select-none items-center rounded-xs px-2 py-1.5 text-sm outline-hidden hover:bg-accent hover:text-accent-foreground"
            onClick={handleDownload}
          >
            Download image
          </button>
        </div>
      ) : null}
      <DialogPrimitive.Root open={lightboxOpen} onOpenChange={setLightboxOpen}>
        <DialogPrimitive.Portal>
          <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
          <DialogPrimitive.Content
            className="fixed inset-0 z-50 flex items-center justify-center p-8"
            onPointerDownOutside={(e) => e.preventDefault()}
            onInteractOutside={(e) => e.preventDefault()}
          >
            <DialogPrimitive.Title className="sr-only">
              {alt || "Image preview"}
            </DialogPrimitive.Title>
            <DialogPrimitive.Description className="sr-only">
              Full-size image preview. Press Escape or click outside the image
              to close.
            </DialogPrimitive.Description>
            {/* Clicking anywhere except the image closes the dialog. */}
            <DialogPrimitive.Close
              className="absolute inset-0 cursor-default"
              aria-label="Close lightbox"
            />
            <img
              alt={alt}
              className="relative max-h-[90vh] max-w-[90vw] rounded-lg object-contain"
              src={resolvedSrc}
              onContextMenuCapture={handleContextMenu}
            />
            <DialogPrimitive.Close className="absolute right-4 top-4 rounded-full bg-black/50 p-2 text-white/80 transition-colors hover:bg-black/70 hover:text-white focus:outline-hidden focus:ring-2 focus:ring-white/30">
              <svg
                aria-hidden="true"
                xmlns="http://www.w3.org/2000/svg"
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
              <span className="sr-only">Close</span>
            </DialogPrimitive.Close>
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>
    </>
  );
}

function getReactNodeText(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }

  if (Array.isArray(node)) {
    return node.map(getReactNodeText).join("");
  }

  if (React.isValidElement<{ children?: React.ReactNode }>(node)) {
    return getReactNodeText(node.props.children);
  }

  return "";
}

function getCodeBlockText(children: React.ReactNode) {
  return getReactNodeText(children).replace(/\n$/, "");
}

function InlineEmojiPopover({
  alt,
  resolvedSrc,
}: {
  alt: string | undefined;
  resolvedSrc: string;
}) {
  const [open, setOpen] = React.useState(false);
  const openTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const label = alt?.trim() || "Custom emoji";

  const clearTimers = React.useCallback(() => {
    if (openTimeout.current) {
      clearTimeout(openTimeout.current);
      openTimeout.current = null;
    }
    if (closeTimeout.current) {
      clearTimeout(closeTimeout.current);
      closeTimeout.current = null;
    }
  }, []);

  const handleMouseEnter = React.useCallback(() => {
    clearTimers();
    openTimeout.current = setTimeout(() => setOpen(true), 200);
  }, [clearTimers]);

  const scheduleClose = React.useCallback(() => {
    clearTimers();
    closeTimeout.current = setTimeout(() => setOpen(false), 150);
  }, [clearTimers]);

  const handleFocus = React.useCallback(() => {
    clearTimers();
    setOpen(true);
  }, [clearTimers]);

  React.useEffect(() => clearTimers, [clearTimers]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex border-0 bg-transparent p-0 align-middle text-inherit"
          aria-label={label}
          onMouseEnter={handleMouseEnter}
          onMouseLeave={scheduleClose}
          onFocus={handleFocus}
          onBlur={scheduleClose}
        >
          <img
            alt={alt}
            title={label}
            src={resolvedSrc}
            data-custom-emoji=""
            className="mx-px inline-block h-[1.25em] w-auto max-w-none align-middle"
            draggable={false}
            onContextMenu={(e) => e.preventDefault()}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="center"
        side="top"
        sideOffset={6}
        className="w-auto min-w-32 max-w-56 rounded-xl p-3"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={scheduleClose}
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        <div className="flex flex-col items-center text-center">
          <div className="mb-2 flex h-14 w-14 items-center justify-center">
            <img
              alt={alt}
              src={resolvedSrc}
              className="inline-block h-12 w-12 object-contain"
              draggable={false}
            />
          </div>
          <div className="max-w-[12rem] text-balance text-sm font-semibold leading-snug text-popover-foreground">
            {label}
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function MarkdownCodeBlock({
  children,
  language,
}: {
  children?: React.ReactNode;
  language?: string;
}) {
  const [isCopying, setIsCopying] = React.useState(false);
  const code = React.useMemo(() => getCodeBlockText(children), [children]);

  const handleCopy = React.useCallback(
    async (event: React.MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();
      setIsCopying(true);

      try {
        await copyCodeBlockToClipboard(code);
        toast.success("Copied code to clipboard");
      } catch (error) {
        console.error("Failed to copy code block", error);
        toast.error("Failed to copy code");
      } finally {
        setIsCopying(false);
      }
    },
    [code],
  );

  return (
    <div className="group relative" data-code-block="">
      <pre className="max-h-[400px] overflow-x-auto overflow-y-auto rounded-xl border border-border/70 bg-muted/60 px-3 py-1.5 pr-12 shadow-xs">
        {language && (
          <div className="mb-1 text-xs text-muted-foreground/70">
            {language}
          </div>
        )}
        {children}
      </pre>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            aria-label="Copy code block"
            className="absolute right-2 top-2 h-7 w-7 bg-background/80 text-muted-foreground opacity-0 shadow-xs ring-1 ring-border/60 backdrop-blur-sm transition-opacity hover:bg-background hover:text-foreground hover:opacity-100 focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 disabled:opacity-60"
            disabled={isCopying}
            onClick={handleCopy}
            size="icon"
            type="button"
            variant="ghost"
          >
            <Copy className="h-4 w-4" />
            <span className="sr-only">Copy code block</span>
          </Button>
        </TooltipTrigger>
        <TooltipContent>Copy code</TooltipContent>
      </Tooltip>
    </div>
  );
}

/** Human-readable byte size: "820 B", "12.4 KB", "3.1 MB". */
function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = bytes / 1024;
  let i = 0;
  while (size >= 1024 && i < units.length - 1) {
    size /= 1024;
    i += 1;
  }
  return `${size < 10 ? size.toFixed(1) : Math.round(size)} ${units[i]}`;
}

/**
 * File card for a generic (non-image, non-video) attachment: icon, filename,
 * size, and a download action.
 *
 * Downloads go through the native `download_file` Tauri command (HTTP inside
 * the app's tunnel + a save dialog), not a plain `<a download>` link. A bare
 * link navigates the webview to the blob URL, which escapes to the OS browser
 * and gets bounced to a corporate CDN interstitial ("browser not supported").
 * The native command mirrors the image-download path.
 */
function FileCard({
  href,
  filename,
  size,
}: {
  href: string;
  filename: string;
  size?: number;
}) {
  const sizeLabel = size != null ? formatFileSize(size) : "";
  return (
    <button
      type="button"
      onClick={() => {
        invokeTauri("download_file", { url: href, filename }).catch(
          (err: unknown) => {
            const msg = err instanceof Error ? err.message : "Download failed";
            toast.error(msg);
          },
        );
      }}
      data-testid="file-card"
      className="my-1 inline-flex max-w-sm items-center gap-3 rounded-xl border border-border/70 bg-muted/40 px-3 py-2 text-left no-underline transition-colors hover:bg-muted/70"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-background text-muted-foreground">
        <FileText className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {filename}
        </span>
        {sizeLabel ? (
          <span className="block text-xs text-muted-foreground">
            {sizeLabel}
          </span>
        ) : null}
      </span>
      <Download className="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

function SyntaxHighlightedCode({
  code,
  language,
  ...props
}: {
  code: string;
  language: string;
} & React.ComponentProps<"code">) {
  const { themeName } = useTheme();
  const [loadedKey, setLoadedKey] = React.useState(0);

  React.useEffect(() => {
    let cancelled = false;
    async function loadAssets() {
      try {
        await ensureHighlighter();
        if (!shikiHighlighter || cancelled) return;
        let loaded = false;
        if (!loadedLangs.has(language)) {
          if (loadedLangs.size >= MAX_LOADED_LANGUAGES) return;
          try {
            await shikiHighlighter.loadLanguage(language as BundledLanguage);
            loadedLangs.add(language);
            loaded = true;
          } catch {
            return;
          }
        }
        if (!loadedThemes.has(themeName as string)) {
          try {
            await shikiHighlighter.loadTheme(themeName as BundledTheme);
            loadedThemes.add(themeName as string);
            loaded = true;
          } catch {
            return;
          }
        }
        if (loaded && !cancelled) setLoadedKey((k) => k + 1);
      } catch {
        /* ignore */
      }
    }
    if (!loadedLangs.has(language) || !loadedThemes.has(themeName as string)) {
      loadAssets();
    }
    return () => {
      cancelled = true;
    };
  }, [language, themeName]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: loadedKey intentionally triggers re-memoization after async asset loading
  const tokens = React.useMemo(() => {
    if (
      !shikiHighlighter ||
      !loadedLangs.has(language) ||
      !loadedThemes.has(themeName as string)
    )
      return null;
    if ((code.match(/\n/g) || []).length > MAX_HIGHLIGHT_LINES) return null;
    const cacheKey = `${language}:${themeName}:${code}`;
    const cached = tokenCache.get(cacheKey);
    if (cached) return cached;
    try {
      const result = shikiHighlighter.codeToTokens(code, {
        lang: language as BundledLanguage,
        theme: themeName as BundledTheme,
      });
      if (tokenCache.size >= MAX_CACHE_ENTRIES) {
        const firstKey = tokenCache.keys().next().value;
        if (firstKey !== undefined) tokenCache.delete(firstKey);
      }
      tokenCache.set(cacheKey, result.tokens);
      return result.tokens;
    } catch {
      return null;
    }
  }, [code, language, themeName, loadedKey]);

  const codeClassName = CODE_BLOCK_CLASS;

  if (!tokens) {
    const lines = code.split("\n");
    return (
      <code {...props} className={codeClassName}>
        {lines.map((line, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
          <span key={i} data-line="">
            {line}
          </span>
        ))}
      </code>
    );
  }

  return (
    <code {...props} className={codeClassName}>
      {tokens.map((line, lineIdx) => {
        const lineText = line.map((t) => t.content).join("");
        const isAdd = DIFF_ADD_RE.test(lineText);
        const isRemove = DIFF_REMOVE_RE.test(lineText);
        const diffClass = isAdd
          ? "code-line-diff-add"
          : isRemove
            ? "code-line-diff-remove"
            : undefined;

        const renderedTokens =
          isAdd || isRemove
            ? stripDiffMarker(line, isAdd ? DIFF_ADD_RE : DIFF_REMOVE_RE)
            : line;

        return (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tokens are positional and never reordered
            key={lineIdx}
            data-line=""
            className={diffClass}
          >
            {renderedTokens.map((token, tokenIdx) => (
              <span
                // biome-ignore lint/suspicious/noArrayIndexKey: tokens are positional and never reordered
                key={tokenIdx}
                style={token.color ? { color: token.color } : undefined}
              >
                {token.content}
              </span>
            ))}
          </span>
        );
      })}
    </code>
  );
}
function createMarkdownComponents(
  variant: MarkdownVariant,
  channels: Channel[],
  onOpenChannel: (channelId: string) => void,
  onOpenMessageLink: (link: ParsedMessageLink) => void,
  imetaByUrl?: ImetaLookup,
  mentionPubkeysByName?: Record<string, string>,
  agentMentionPubkeysByName?: Record<string, string>,
  interactive = true,
): Components {
  const paragraphClassName =
    variant === "tight"
      ? "leading-5"
      : variant === "compact"
        ? "leading-6"
        : "leading-7";
  const listItemClassName =
    variant === "tight" ? "my-0.5 [&_p]:inline" : "my-1 [&_p]:inline";
  const listClassName =
    variant === "tight"
      ? "space-y-0.5 pl-6 marker:text-muted-foreground"
      : "space-y-1 pl-6 marker:text-muted-foreground";

  return {
    a: ({ children, href, ...props }) => {
      if (!interactive) {
        return <span className="font-medium text-current">{children}</span>;
      }

      // Generic file attachment: a `[filename](url)` link whose href matches an
      // imeta entry with a non-image, non-video MIME. Render a download card
      // instead of a plain link. (Media uses the `img` renderer, not this path.)
      const card = resolveFileCard(
        href ? imetaByUrl?.get(href) : undefined,
        href,
        getReactNodeText(children),
      );
      if (card) {
        return (
          <FileCard
            href={card.href}
            filename={card.filename}
            size={card.size}
          />
        );
      }

      // Intercept `buzz://message?channel=…&id=…` links so a click navigates
      // in-app instead of opening the URL in the OS browser. http(s) links
      // continue to use the existing target="_blank" behavior.
      if (isMessageLink(href)) {
        const parsed = parseMessageLink(href ?? "");
        if (parsed.ok) {
          const target = parsed.value;
          return (
            <a
              {...props}
              className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80 cursor-pointer"
              href={href}
              onClick={(event) => {
                event.preventDefault();
                onOpenMessageLink(target);
              }}
            >
              {children}
            </a>
          );
        }
        // Malformed message deep link — fall through to the default
        // anchor (renders as a normal external link).
      }
      return (
        <a
          {...props}
          className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80"
          href={href}
          rel="noreferrer"
          target="_blank"
        >
          {children}
        </a>
      );
    },
    blockquote: ({ children }) => (
      <blockquote className="border-l-2 border-border pl-4 italic text-muted-foreground [&>*:first-child]:mt-0 [&>*+*]:mt-2">
        {children}
      </blockquote>
    ),
    br: () => <br />,
    code: ({ children, className, ...props }: React.ComponentProps<"code">) => {
      const rawCode = String(children);
      const code = rawCode.replace(/\n$/, "");
      const isFencedCodeBlock =
        typeof className === "string" && className.includes("language-");

      if (isFencedCodeBlock || rawCode.endsWith("\n") || code.includes("\n")) {
        const language = extractLanguage(className);

        if (language) {
          return (
            <SyntaxHighlightedCode code={code} language={language} {...props} />
          );
        }

        const lines = code.split("\n");
        return (
          <code {...props} className={CODE_BLOCK_CLASS}>
            {lines.map((line, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
              <span key={i} data-line="">
                {line}
              </span>
            ))}
          </code>
        );
      }

      return (
        <code
          {...props}
          className={cn(
            "rounded-md bg-muted px-1.5 py-0.5 font-mono text-[13px] text-foreground",
            className,
          )}
        >
          {children}
        </code>
      );
    },
    h1: ({ children }) => (
      <h1 className="text-xl font-semibold leading-8 tracking-tight">
        {children}
      </h1>
    ),
    h2: ({ children }) => (
      <h2 className="text-lg font-semibold leading-7 tracking-tight">
        {children}
      </h2>
    ),
    h3: ({ children }) => (
      <h3 className="text-base font-semibold leading-6 tracking-tight">
        {children}
      </h3>
    ),
    hr: () => <hr className="border-border/80" />,
    img: ({ alt, src }) => {
      const resolvedSrc = src ? rewriteRelayUrl(src) : src;
      if (!interactive) {
        const fallbackLabel = resolvedSrc?.endsWith(".mp4")
          ? "Video attachment"
          : "Image attachment";
        return <span>{alt?.trim() || fallbackLabel}</span>;
      }

      if (resolvedSrc?.endsWith(".mp4")) {
        const entry = src ? imetaByUrl?.get(src) : undefined;
        return (
          <span data-block-media="">
            <MarkdownVideoPlayer
              key={src ?? resolvedSrc}
              alt={alt}
              entry={entry}
              resolvedSrc={resolvedSrc}
              src={src}
            />
          </span>
        );
      }
      return (
        <span data-block-media="" className="block">
          <ImageBlock alt={alt} resolvedSrc={resolvedSrc} src={src} />
        </span>
      );
    },
    li: ({ children }) => <li className={listItemClassName}>{children}</li>,
    ol: ({ children }) => (
      <ol className={cn("list-decimal", listClassName)}>{children}</ol>
    ),
    p: ({ children }) => {
      // Detect image-only paragraphs (images + <br> from remarkBreaks).
      // Multi-image: render as a 2-column grid gallery.
      // Single media: render as a plain <div> to avoid invalid <p><div> nesting
      // (the img component returns block-level wrappers for lightbox/video).
      const childArray = React.Children.toArray(children);
      const { imageChildren } = classifyChildren(childArray);

      if (isImageOnlyParagraph(childArray)) {
        return (
          <div className="mt-1 grid max-w-lg grid-cols-2 gap-1.5 [&_br]:hidden [&_[data-block-media]]:mt-0 [&_[data-block-media]]:max-w-none [&_img]:mt-0 [&_img]:w-full [&_img]:max-w-full">
            {imageChildren}
          </div>
        );
      }

      if (hasBlockMedia(childArray)) {
        return <div className={paragraphClassName}>{children}</div>;
      }

      return <p className={paragraphClassName}>{children}</p>;
    },
    pre: ({ children }) => {
      if (!interactive) return <span>{children}</span>;
      let language = "";
      React.Children.forEach(children, (child) => {
        if (
          React.isValidElement<Record<string, unknown>>(child) &&
          typeof child.props?.className === "string"
        ) {
          language = extractLanguage(child.props.className);
        }
      });
      return (
        <MarkdownCodeBlock language={language}>{children}</MarkdownCodeBlock>
      );
    },
    strong: ({ children }) => (
      <strong className="font-semibold">{children}</strong>
    ),
    table: ({ children }) => (
      <div
        className="overflow-x-auto rounded-2xl border border-border/70"
        data-table-block=""
      >
        <table className="w-full border-collapse text-left text-sm">
          {children}
        </table>
      </div>
    ),
    td: ({ children }) => (
      <td className="border-t border-border/70 px-3 py-2 align-top">
        {children}
      </td>
    ),
    th: ({ children }) => (
      <th className="bg-muted/60 px-3 py-2 font-semibold text-foreground">
        {children}
      </th>
    ),
    ul: ({ children }) => (
      <ul className={cn("list-disc", listClassName)}>{children}</ul>
    ),
    mention: ({ children }: { children?: React.ReactNode }) => {
      const mentionText = String(children ?? "");
      const mentionName = mentionText.replace(/^@/, "").trim().toLowerCase();
      const pubkey = mentionPubkeysByName?.[mentionName];
      const isAgentMention =
        pubkey !== undefined &&
        agentMentionPubkeysByName?.[mentionName] === pubkey;
      const renderedMentionText = isAgentMention
        ? mentionText.replace(/^@/, "")
        : children;
      const mentionNode = (
        <span
          data-mention=""
          className={cn(
            "cursor-pointer",
            MENTION_CHIP_BASE_CLASSES,
            MENTION_CHIP_HOVER_CLASSES,
            isAgentMention && "agent-mention-highlight",
          )}
        >
          {renderedMentionText}
        </span>
      );

      if (!interactive) {
        return mentionNode;
      }

      return pubkey ? (
        <UserProfilePopover pubkey={pubkey} triggerElement="span">
          {mentionNode}
        </UserProfilePopover>
      ) : (
        mentionNode
      );
    },
    emoji: ({ src, alt }: { src?: string; alt?: string }) => {
      const resolvedSrc = src ? rewriteRelayUrl(src) : src;
      if (!resolvedSrc) {
        return <span>{alt}</span>;
      }
      if (!interactive) {
        return <span>{alt}</span>;
      }
      return <InlineEmojiPopover alt={alt} resolvedSrc={resolvedSrc} />;
    },
    "channel-link": ({ children }: { children?: React.ReactNode }) => {
      const text = String(children ?? "");
      const channelName = text.startsWith("#") ? text.slice(1) : text;
      const channel = channels.find(
        (c) =>
          c.channelType !== "dm" &&
          c.name.toLowerCase() === channelName.toLowerCase(),
      );

      if (channel && interactive) {
        return (
          <button
            type="button"
            data-channel-link=""
            aria-label={`Open channel ${channelName}`}
            className="rounded-md bg-primary/15 px-1 py-0.5 text-sm font-medium text-primary cursor-pointer hover:bg-primary/25 transition-colors"
            onClick={() => {
              onOpenChannel(channel.id);
            }}
          >
            {children}
          </button>
        );
      }

      return (
        <span
          data-channel-link=""
          className="rounded-md bg-primary/15 px-1 py-0.5 text-sm text-primary"
        >
          {children}
        </span>
      );
    },
    "message-link": ({ children }: { children?: React.ReactNode }) => {
      const href = String(children ?? "");
      const parsed = parseMessageLink(href);
      if (!parsed.ok) {
        // Malformed `buzz://message?…` — render the raw URL as plain text
        // rather than a misleading clickable pill.
        return <span data-message-link="">{href}</span>;
      }

      const { channelId, messageId } = parsed.value;
      const channel = channels.find((c) => c.id === channelId);
      const channelLabel = channel?.name ?? "channel";
      const shortId = messageId.slice(0, 6);

      if (!interactive) {
        return (
          <span data-message-link="">
            #{channelLabel} · {shortId}
          </span>
        );
      }

      return (
        <button
          type="button"
          data-message-link=""
          aria-label={`Open message in ${channelLabel}`}
          title={href}
          className="rounded-md bg-primary/15 px-1 py-0.5 text-sm font-medium text-primary cursor-pointer hover:bg-primary/25 transition-colors"
          onClick={() => {
            onOpenMessageLink(parsed.value);
          }}
        >
          #{channelLabel} · {shortId}
        </button>
      );
    },
  } as Components;
}

function MarkdownInner({
  channelNames,
  className,
  compact = false,
  content,
  customEmoji,
  imetaByUrl,
  interactive = true,
  agentMentionPubkeysByName,
  mentionNames,
  mentionPubkeysByName,
  searchQuery,
  tight = false,
  videoReviewContext,
}: MarkdownProps) {
  const variant: MarkdownVariant = tight
    ? "tight"
    : compact
      ? "compact"
      : "default";
  const { channels: rawChannels } = useChannelNavigation();
  const channels = useStableArray(rawChannels);
  const { goChannel } = useAppNavigation();

  const components = React.useMemo(
    () =>
      createMarkdownComponents(
        variant,
        channels,
        (channelId) => {
          void goChannel(channelId);
        },
        (link) => {
          // Always route through `goChannel` with `messageId` set: the
          // channel route already handles scroll-into-view + highlight via
          // `useTimelineScrollManager` + `getEventById` backfill, and works
          // for both stream-message replies and forum threads. Detecting
          // "the thread root is a forum post" up front would require an
          // event lookup we don't currently have synchronously; the brief
          // explicitly allows skipping that detection and falling through.
          void goChannel(link.channelId, {
            messageId: link.messageId,
            threadRootId: link.threadRootId,
          });
        },
        imetaByUrl,
        mentionPubkeysByName,
        agentMentionPubkeysByName,
        interactive,
      ),
    [
      goChannel,
      variant,
      channels,
      imetaByUrl,
      mentionPubkeysByName,
      agentMentionPubkeysByName,
      interactive,
    ],
  );

  // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
  const remarkPlugins = React.useMemo<any[]>(
    () => [
      remarkGfm,
      remarkBreaks,
      remarkMessageLinks,
      [remarkMentions, { mentionNames }],
      [remarkChannelLinks, { channelNames }],
      [remarkCustomEmoji, { customEmoji }],
    ],
    [mentionNames, channelNames, customEmoji],
  );

  // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
  const rehypePlugins = React.useMemo<any[]>(() => {
    // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
    const plugins: any[] = [rehypeImageGallery];
    if (searchQuery && searchQuery.trim().length >= 2) {
      plugins.push([rehypeSearchHighlight, { query: searchQuery }]);
    }
    return plugins;
  }, [searchQuery]);

  let processedContent = content;

  if (/^(?:\s{2}\n)+/.test(content)) {
    processedContent = `\u200B${processedContent}`;
  }

  if (/(?:\s{2}\n)+$/.test(content)) {
    processedContent = `${processedContent}\u200B`;
  }

  const markdownNode = (
    <ReactMarkdown
      components={components}
      remarkPlugins={remarkPlugins}
      rehypePlugins={rehypePlugins}
      urlTransform={messageLinkUrlTransform}
    >
      {processedContent}
    </ReactMarkdown>
  );

  return (
    <div
      className={cn(
        tight
          ? [
              "max-w-none break-words text-sm leading-5 text-foreground/90",
              // Reset first/last
              "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
              // Base owl: p+p, list+p, etc.
              "[&>*+*]:mt-2",
              // Headings: flat push/pull — size does the hierarchy work
              "[&>*+h1]:mt-2.5 [&>*+h2]:mt-2.5 [&>*+h3]:mt-2.5",
              "[&>h1+*]:mt-0.5 [&>h2+*]:mt-0.5 [&>h3+*]:mt-0.5",
              // Blockquotes: breathe above and below
              "[&>*+blockquote]:mt-3 [&>blockquote+*]:mt-3",
              // Code blocks: breathe above and below
              "[&>*+[data-code-block]]:mt-3 [&>[data-code-block]+*]:mt-3",
              // Tables: breathe above and below
              "[&>*+[data-table-block]]:mt-3 [&>[data-table-block]+*]:mt-3",
              // hr: clear section divider
              "[&>*+hr]:mt-3.5 [&>hr+*]:mt-3.5",
              // Lists after paragraphs: tighter to feel related
              "[&>p+ul]:mt-1 [&>p+ol]:mt-1 [&>div+ul]:mt-1 [&>div+ol]:mt-1",
            ].join(" ")
          : compact
            ? [
                "max-w-none break-words text-[15px] leading-6 text-foreground/90",
                "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
                "[&>*+*]:mt-2",
                "[&>*+h1]:mt-3 [&>*+h2]:mt-3 [&>*+h3]:mt-3",
                "[&>h1+*]:mt-0.5 [&>h2+*]:mt-0.5 [&>h3+*]:mt-0.5",
                "[&>*+blockquote]:mt-3 [&>blockquote+*]:mt-3",
                "[&>*+[data-code-block]]:mt-3 [&>[data-code-block]+*]:mt-3",
                "[&>*+[data-table-block]]:mt-3 [&>[data-table-block]+*]:mt-3",
                "[&>*+hr]:mt-3.5 [&>hr+*]:mt-3.5",
                "[&>p+ul]:mt-1 [&>p+ol]:mt-1 [&>div+ul]:mt-1 [&>div+ol]:mt-1",
              ].join(" ")
            : [
                "max-w-none break-words text-sm leading-7 text-foreground/90",
                "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
                "[&>*+*]:mt-3",
                "[&>*+h1]:mt-3.5 [&>*+h2]:mt-3.5 [&>*+h3]:mt-3.5",
                "[&>h1+*]:mt-0.5 [&>h2+*]:mt-0.5 [&>h3+*]:mt-0.5",
                "[&>*+blockquote]:mt-3.5 [&>blockquote+*]:mt-3.5",
                "[&>*+[data-code-block]]:mt-3.5 [&>[data-code-block]+*]:mt-3.5",
                "[&>*+[data-table-block]]:mt-3.5 [&>[data-table-block]+*]:mt-3.5",
                "[&>*+hr]:mt-4 [&>hr+*]:mt-4",
                "[&>p+ul]:mt-1.5 [&>p+ol]:mt-1.5 [&>div+ul]:mt-1.5 [&>div+ol]:mt-1.5",
              ].join(" "),
        className,
      )}
    >
      <VideoReviewMarkdownContext.Provider value={videoReviewContext}>
        {markdownNode}
      </VideoReviewMarkdownContext.Provider>
    </div>
  );
}

export const Markdown = React.memo(
  MarkdownInner,
  (prev, next) =>
    prev.content === next.content &&
    prev.className === next.className &&
    prev.compact === next.compact &&
    prev.customEmoji === next.customEmoji &&
    prev.interactive === next.interactive &&
    prev.tight === next.tight &&
    prev.agentMentionPubkeysByName === next.agentMentionPubkeysByName &&
    prev.mentionPubkeysByName === next.mentionPubkeysByName &&
    shallowArrayEqual(prev.mentionNames, next.mentionNames) &&
    shallowArrayEqual(prev.channelNames, next.channelNames) &&
    prev.imetaByUrl === next.imetaByUrl &&
    prev.searchQuery === next.searchQuery &&
    prev.videoReviewContext === next.videoReviewContext,
);

Markdown.displayName = "Markdown";
