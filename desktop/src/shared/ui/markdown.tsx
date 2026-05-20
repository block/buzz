import * as React from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { invokeTauri } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import rehypeImageGallery from "@/shared/lib/rehypeImageGallery";
import rehypeSearchHighlight from "@/shared/lib/rehypeSearchHighlight";
import remarkChannelLinks from "@/shared/lib/remarkChannelLinks";
import remarkMentions from "@/shared/lib/remarkMentions";

import {
  classifyChildren,
  hasBlockMedia,
  isImageOnlyParagraph,
  shallowArrayEqual,
} from "./markdownUtils";
import { VideoPlayer } from "./VideoPlayer";

type ImetaLookup = Map<string, { image?: string; thumb?: string }>;

type MarkdownProps = {
  channelNames?: string[];
  className?: string;
  compact?: boolean;
  content: string;
  imetaByUrl?: ImetaLookup;
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  searchQuery?: string;
  tight?: boolean;
};

type MarkdownVariant = "default" | "compact" | "tight";

function ImageContextMenu({
  children,
  src,
}: {
  children: React.ReactNode;
  src: string | undefined;
}) {
  const [menu, setMenu] = React.useState<{ x: number; y: number } | null>(null);

  React.useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [menu]);

  return (
    <>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: context menu handler on image wrapper */}
      <div
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
      >
        {children}
      </div>
      {menu && src && (
        <div
          className="fixed z-[100] min-w-[160px] rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
          style={{ left: menu.x, top: menu.y }}
        >
          <button
            type="button"
            className="flex w-full cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground"
            onClick={() => {
              setMenu(null);
              invokeTauri("download_image", { url: src }).catch(
                (err: unknown) => {
                  const msg =
                    err instanceof Error ? err.message : "Download failed";
                  toast.error(msg);
                },
              );
            }}
          >
            Download image
          </button>
        </div>
      )}
    </>
  );
}

function createMarkdownComponents(
  variant: MarkdownVariant,
  channels: Channel[],
  onOpenChannel: (channelId: string) => void,
  imetaByUrl?: ImetaLookup,
  mentionPubkeysByName?: Record<string, string>,
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
    a: ({ children, href, ...props }) => (
      <a
        {...props}
        className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80"
        href={href}
        rel="noreferrer"
        target="_blank"
      >
        {children}
      </a>
    ),
    blockquote: ({ children }) => (
      <blockquote className="border-l-2 border-border pl-4 italic text-muted-foreground">
        {children}
      </blockquote>
    ),
    br: () => <br />,
    code: ({
      children,
      className,
      ...props
    }: React.ComponentProps<"code"> & { inline?: boolean }) => {
      const code = String(children).replace(/\n$/, "");
      const isBlock =
        typeof className === "string" && className.includes("language-")
          ? true
          : code.includes("\n");

      if (isBlock) {
        return (
          <code
            {...props}
            className={cn(
              "block min-w-full whitespace-pre font-mono text-[13px] leading-6 text-foreground",
              className,
            )}
          >
            {code}
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
      <h1 className="text-lg font-semibold tracking-tight">{children}</h1>
    ),
    h2: ({ children }) => (
      <h2 className="text-base font-semibold tracking-tight">{children}</h2>
    ),
    h3: ({ children }) => (
      <h3 className="font-semibold tracking-tight">{children}</h3>
    ),
    hr: () => <hr className="border-border/80" />,
    img: ({ alt, src }) => {
      const resolvedSrc = src ? rewriteRelayUrl(src) : src;
      if (resolvedSrc?.endsWith(".mp4")) {
        // Look up poster frame from imeta tags (NIP-71 `image` field).
        // Fall back to `thumb` for compatibility with older events.
        const entry = src ? imetaByUrl?.get(src) : undefined;
        const posterUrl = entry?.image ?? entry?.thumb;
        const resolvedPoster = posterUrl
          ? rewriteRelayUrl(posterUrl)
          : undefined;
        return (
          <VideoPlayer
            key={resolvedSrc}
            src={resolvedSrc}
            poster={resolvedPoster}
          />
        );
      }
      return (
        <ImageContextMenu src={src}>
          <DialogPrimitive.Root>
            <DialogPrimitive.Trigger asChild>
              <div className="mt-1 max-w-sm cursor-pointer transition-opacity hover:opacity-90">
                <img
                  alt={alt}
                  className="max-h-64 max-w-full rounded-xl object-contain"
                  src={resolvedSrc}
                  onContextMenu={(e) => e.preventDefault()}
                />
              </div>
            </DialogPrimitive.Trigger>
            <DialogPrimitive.Portal>
              <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
              <DialogPrimitive.Content
                className="fixed inset-0 z-50 flex items-center justify-center p-8"
                // Let clicks on the backdrop (the content container itself) close the lightbox
                onPointerDownOutside={(e) => e.preventDefault()}
                onInteractOutside={(e) => e.preventDefault()}
              >
                <DialogPrimitive.Title className="sr-only">
                  {alt || "Image preview"}
                </DialogPrimitive.Title>
                <DialogPrimitive.Description className="sr-only">
                  Full-size image preview. Press Escape or click outside the
                  image to close.
                </DialogPrimitive.Description>
                {/* Close region: clicking anywhere except the image closes the dialog */}
                <DialogPrimitive.Close
                  className="absolute inset-0 cursor-default"
                  aria-label="Close lightbox"
                />
                <ImageContextMenu src={src}>
                  <img
                    alt={alt}
                    className="relative max-h-[90vh] max-w-[90vw] rounded-lg object-contain"
                    src={resolvedSrc}
                    onContextMenu={(e) => e.preventDefault()}
                  />
                </ImageContextMenu>
                <DialogPrimitive.Close className="absolute right-4 top-4 rounded-full bg-black/50 p-2 text-white/80 transition-colors hover:bg-black/70 hover:text-white focus:outline-none focus:ring-2 focus:ring-white/30">
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
        </ImageContextMenu>
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
          <div className="mt-1 grid max-w-lg grid-cols-2 gap-1.5 [&_br]:hidden [&_div]:mt-0 [&_div]:max-w-none">
            {imageChildren}
          </div>
        );
      }

      if (hasBlockMedia(childArray)) {
        return <div className={paragraphClassName}>{children}</div>;
      }

      return <p className={paragraphClassName}>{children}</p>;
    },
    pre: ({ children }) => (
      <pre className="overflow-x-auto rounded-xl border border-border/70 bg-muted/60 px-3 py-1.5 shadow-sm">
        {children}
      </pre>
    ),
    strong: ({ children }) => (
      <strong className="font-semibold">{children}</strong>
    ),
    table: ({ children }) => (
      <div className="overflow-x-auto rounded-2xl border border-border/70">
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
      const mentionNode = (
        <span
          data-mention=""
          className="cursor-pointer rounded-md bg-primary/15 px-1 py-0.5 text-sm font-semibold text-primary transition-colors hover:bg-primary/25 hover:text-primary/90"
        >
          {children}
        </span>
      );

      return pubkey ? (
        <UserProfilePopover pubkey={pubkey} triggerElement="span">
          {mentionNode}
        </UserProfilePopover>
      ) : (
        mentionNode
      );
    },
    "channel-link": ({ children }: { children?: React.ReactNode }) => {
      const text = String(children ?? "");
      const channelName = text.startsWith("#") ? text.slice(1) : text;
      const channel = channels.find(
        (c) =>
          c.channelType !== "dm" &&
          c.name.toLowerCase() === channelName.toLowerCase(),
      );

      if (channel) {
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
  } as Components;
}

function MarkdownInner({
  channelNames,
  className,
  compact = false,
  content,
  imetaByUrl,
  mentionNames,
  mentionPubkeysByName,
  searchQuery,
  tight = false,
}: MarkdownProps) {
  const variant: MarkdownVariant = tight
    ? "tight"
    : compact
      ? "compact"
      : "default";
  const { channels } = useChannelNavigation();
  const { goChannel } = useAppNavigation();

  const components = React.useMemo(
    () =>
      createMarkdownComponents(
        variant,
        channels,
        (channelId) => {
          void goChannel(channelId);
        },
        imetaByUrl,
        mentionPubkeysByName,
      ),
    [goChannel, variant, channels, imetaByUrl, mentionPubkeysByName],
  );

  // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
  const remarkPlugins = React.useMemo<any[]>(
    () => [
      remarkGfm,
      remarkBreaks,
      [remarkMentions, { mentionNames }],
      [remarkChannelLinks, { channelNames }],
    ],
    [mentionNames, channelNames],
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
    >
      {processedContent}
    </ReactMarkdown>
  );

  return (
    <div
      className={cn(
        tight
          ? "max-w-none break-words text-sm leading-5 text-foreground/90 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 [&>*]:my-0.5"
          : compact
            ? "max-w-none break-words text-[15px] leading-6 text-foreground/90 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 [&>*]:my-1.5"
            : "max-w-none break-words text-sm leading-7 text-foreground/90 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 [&>*]:my-3",
        className,
      )}
    >
      {markdownNode}
    </div>
  );
}

export const Markdown = React.memo(
  MarkdownInner,
  (prev, next) =>
    prev.content === next.content &&
    prev.className === next.className &&
    prev.compact === next.compact &&
    prev.tight === next.tight &&
    prev.mentionPubkeysByName === next.mentionPubkeysByName &&
    shallowArrayEqual(prev.mentionNames, next.mentionNames) &&
    shallowArrayEqual(prev.channelNames, next.channelNames) &&
    prev.imetaByUrl === next.imetaByUrl &&
    prev.searchQuery === next.searchQuery,
);

Markdown.displayName = "Markdown";
