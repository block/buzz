import {
  Bold,
  Code,
  Hash,
  Image as ImageIcon,
  Italic,
  Link2,
  List,
  ListOrdered,
  Quote,
  Strikethrough,
  UserRound,
  X,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "@/features/channels/ui/channelFormStyles";
import { buildChannelLink } from "@/features/messages/lib/channelLink";
import { uploadMediaFile } from "@/shared/api/tauriMedia";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useMediaProxyPort } from "@/shared/lib/useMediaProxyPort";
import { Button } from "@/shared/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import { Input } from "@/shared/ui/input";
import { ACTION_TRAY_SURFACE_CLASS } from "@/shared/ui/actionTray";
import {
  inlineChipIconClasses,
  MESSAGE_MARKDOWN_CLASS,
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
} from "@/shared/ui/mentionChip";
import { POPOVER_CUSTOM_ENTER_MOTION_CLASS } from "@/shared/ui/popoverSurface";
import { WelcomeChannelChipPicker } from "./WelcomeChannelChipPicker";
type WelcomeInsertType = "link" | "image" | "channel";

type WelcomeInsert = {
  id: string;
  type: WelcomeInsertType;
  title: string;
  url: string;
};

export type WelcomeMessage = {
  text: string;
  inserts: WelcomeInsert[];
};

type CaretPosition = {
  left: number;
  top: number;
};

type DropCaretPosition = CaretPosition & {
  height: number;
};

type ChipEditorPosition = CaretPosition & {
  id: string;
};

type SelectionToolbarPosition = CaretPosition & {
  placement: "top" | "bottom";
};

const NEW_MEMBER_TOKEN = "{{member}}";
const ALL_TOKEN_PATTERN = /\{\{member\}\}|\{\{insert:([^}]+)\}\}/g;

export const DEFAULT_MESSAGE: WelcomeMessage = {
  text: `Welcome, ${NEW_MEMBER_TOKEN}! We’re glad you’re here. Take a look around.\n\nIntroduce yourself in {{insert:choose-channel}}, and visit {{insert:add-link}} to learn more about our community.`,
  inserts: [
    {
      id: "choose-channel",
      type: "channel",
      title: "Choose a channel",
      url: "",
    },
    {
      id: "add-link",
      type: "link",
      title: "Add a link",
      url: "",
    },
  ],
};

function insertToken(id: string) {
  return `{{insert:${id}}}`;
}

type WelcomeChipType = WelcomeInsertType | "member";

function makeChipIcon(type: WelcomeChipType) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("class", "h-3.5 w-3.5 shrink-0");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("viewBox", "0 0 24 24");

  const addPath = (d: string) => {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  };

  if (type === "member") {
    addPath("M20 21a8 8 0 0 0-16 0");
    const circle = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "circle",
    );
    circle.setAttribute("cx", "12");
    circle.setAttribute("cy", "7");
    circle.setAttribute("r", "4");
    svg.append(circle);
  } else if (type === "link") {
    addPath("M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71");
    addPath("M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71");
  } else if (type === "image") {
    addPath("m21 15-5-5L5 21");
    const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
    rect.setAttribute("height", "18");
    rect.setAttribute("rx", "2");
    rect.setAttribute("width", "18");
    rect.setAttribute("x", "3");
    rect.setAttribute("y", "3");
    svg.append(rect);
    const circle = document.createElementNS(
      "http://www.w3.org/2000/svg",
      "circle",
    );
    circle.setAttribute("cx", "8.5");
    circle.setAttribute("cy", "8.5");
    circle.setAttribute("r", "1.5");
    svg.append(circle);
  } else {
    addPath("M4 9h16");
    addPath("M4 15h16");
    addPath("M10 3 8 21");
    addPath("m16 3-2 18");
  }

  return svg;
}

function appendChipContents(
  chip: HTMLElement,
  type: WelcomeChipType,
  label: string,
) {
  chip.append(makeChipIcon(type), document.createTextNode(label));
}

function makeMemberChip(label = "New member") {
  const chip = document.createElement("span");
  chip.className = `${MENTION_CHIP_BASE_CLASSES} ${inlineChipIconClasses("human")}`;
  chip.contentEditable = "false";
  chip.dataset.memberToken = "true";
  chip.append(document.createTextNode(label));
  return chip;
}

function makeInsertChip(insert: WelcomeInsert, interactive = true) {
  const chip = document.createElement("span");
  const iconClass =
    insert.type === "channel"
      ? inlineChipIconClasses("channel")
      : "welcome-message-inline-chip";
  chip.className = [
    MENTION_CHIP_BASE_CLASSES,
    iconClass,
    interactive ? `cursor-pointer ${MENTION_CHIP_HOVER_CLASSES}` : "",
  ]
    .filter(Boolean)
    .join(" ");
  chip.contentEditable = "false";
  chip.dataset.insertId = insert.id;
  if (insert.type === "channel") {
    chip.append(document.createTextNode(insert.title));
  } else {
    appendChipContents(chip, insert.type, insert.title);
  }
  return chip;
}

function makePreviewImage(insert: WelcomeInsert) {
  const image = document.createElement("img");
  image.alt = insert.title;
  image.className =
    "my-1 inline-block h-auto w-auto rounded-2xl object-contain align-top";
  image.dataset.welcomePreviewImage = insert.id;
  image.decoding = "async";
  image.loading = "eager";
  image.src = rewriteRelayUrl(insert.url);
  image.style.maxHeight = "16rem";
  image.style.maxWidth = "min(100%, 24rem)";
  return image;
}

function renderEditorContent(
  editor: HTMLElement,
  text: string,
  inserts: WelcomeInsert[],
  options: {
    interactive?: boolean;
    mediaProxyPort?: number | null;
    memberLabel?: string;
    renderImages?: boolean;
  } = {},
) {
  const {
    interactive = true,
    mediaProxyPort,
    memberLabel = "New member",
    renderImages = false,
  } = options;
  void mediaProxyPort;
  const template = document.createElement("template");
  template.innerHTML = text;
  editor.replaceChildren(template.content.cloneNode(true));
  const insertsById = new Map(inserts.map((insert) => [insert.id, insert]));
  const walker = document.createTreeWalker(editor, NodeFilter.SHOW_TEXT);
  const textNodes: Text[] = [];
  while (walker.nextNode()) textNodes.push(walker.currentNode as Text);

  for (const textNode of textNodes) {
    const value = textNode.textContent ?? "";
    if (!value.includes("{{")) continue;
    const fragment = document.createDocumentFragment();
    let cursor = 0;
    for (const match of value.matchAll(ALL_TOKEN_PATTERN)) {
      const offset = match.index ?? cursor;
      fragment.append(document.createTextNode(value.slice(cursor, offset)));
      if (match[0] === NEW_MEMBER_TOKEN) {
        fragment.append(makeMemberChip(memberLabel));
      } else {
        const insert = insertsById.get(match[1]);
        if (insert) {
          fragment.append(
            renderImages && insert.type === "image" && insert.url
              ? makePreviewImage(insert)
              : makeInsertChip(insert, interactive),
          );
        }
      }
      cursor = offset + match[0].length;
    }
    fragment.append(document.createTextNode(value.slice(cursor)));
    textNode.replaceWith(fragment);
  }
}

function readEditorContent(root: HTMLElement): string {
  const clone = root.cloneNode(true) as HTMLElement;
  for (const anchor of clone.querySelectorAll<HTMLElement>(
    "[data-caret-anchor='true']",
  )) {
    anchor.remove();
  }
  for (const member of clone.querySelectorAll<HTMLElement>(
    "[data-member-token='true']",
  )) {
    member.replaceWith(document.createTextNode(NEW_MEMBER_TOKEN));
  }
  for (const insert of clone.querySelectorAll<HTMLElement>(
    "[data-insert-id]",
  )) {
    const id = insert.dataset.insertId;
    insert.replaceWith(document.createTextNode(id ? insertToken(id) : ""));
  }
  return clone.innerHTML;
}

function InlineChipEditor({
  channels,
  insert,
  onChange,
  onClose,
  onRemove,
  position,
}: {
  channels: Channel[];
  insert: WelcomeInsert;
  onChange: (insert: WelcomeInsert) => void;
  onClose: () => void;
  onRemove: () => void;
  position: CaretPosition;
}) {
  if (insert.type === "channel") {
    return (
      <WelcomeChannelChipPicker
        channels={channels}
        insert={insert}
        onClose={onClose}
        onRemove={onRemove}
        onSelect={(channel) => {
          onChange({
            ...insert,
            title: channel.name,
            url: buildChannelLink(channel.id),
          });
          onClose();
        }}
        position={position}
      />
    );
  }

  return (
    <div
      aria-label={`Edit ${insert.type}`}
      className={cn(
        "absolute z-30 w-80 space-y-3 rounded-xl p-4",
        ACTION_TRAY_SURFACE_CLASS,
        POPOVER_CUSTOM_ENTER_MOTION_CLASS,
      )}
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }}
      role="dialog"
      style={{
        left: position.left,
        top: position.top,
      }}
    >
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm font-semibold leading-none">
          {insert.type === "link"
            ? "Link"
            : insert.type === "image"
              ? "Image"
              : "Channel"}
        </p>
        <Button
          aria-label="Close chip editor"
          className="h-8 w-8"
          onClick={onClose}
          size="icon"
          type="button"
          variant="ghost"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className={CHANNEL_FORM_FIELD_SHELL_CLASS}>
        <Input
          aria-label={insert.type === "link" ? "Link title" : "Image alt text"}
          className={cn("h-10", CHANNEL_FORM_FIELD_CONTROL_CLASS)}
          onChange={(event) =>
            onChange({ ...insert, title: event.target.value })
          }
          placeholder={
            insert.type === "link" ? "Community guide" : "Team photo"
          }
          value={insert.title}
        />
      </div>
      <div className={cn("relative", CHANNEL_FORM_FIELD_SHELL_CLASS)}>
        <span className="pointer-events-none absolute inset-y-0 left-2.5 z-10 flex items-center text-muted-foreground">
          {insert.type === "link" ? (
            <Link2 className="h-4 w-4" />
          ) : (
            <ImageIcon className="h-4 w-4" />
          )}
        </span>
        <Input
          aria-label={
            insert.type === "link" ? "Link destination" : "Image source"
          }
          autoCapitalize="off"
          className={cn("h-10 pl-8", CHANNEL_FORM_FIELD_CONTROL_CLASS)}
          onChange={(event) => onChange({ ...insert, url: event.target.value })}
          placeholder={
            insert.type === "link"
              ? "https://example.com"
              : "https://example.com/image.png"
          }
          spellCheck={false}
          value={insert.url}
        />
      </div>
      <Button
        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
        onClick={onRemove}
        size="sm"
        type="button"
        variant="ghost"
      >
        Remove chip
      </Button>
    </div>
  );
}

function WelcomeSelectionToolbar({
  onFormat,
  onLink,
  position,
}: {
  onFormat: (
    command:
      | "bold"
      | "formatBlock"
      | "insertOrderedList"
      | "insertUnorderedList"
      | "italic"
      | "strikeThrough",
    value?: string,
  ) => void;
  onLink: () => void;
  position: SelectionToolbarPosition;
}) {
  const items = [
    { icon: Bold, label: "Bold", command: "bold" as const },
    { icon: Italic, label: "Italic", command: "italic" as const },
    {
      icon: Strikethrough,
      label: "Strikethrough",
      command: "strikeThrough" as const,
    },
    {
      icon: Code,
      label: "Code block",
      command: "formatBlock" as const,
      value: "pre",
    },
    { icon: Link2, label: "Link", action: onLink },
    {
      icon: List,
      label: "Bullet list",
      command: "insertUnorderedList" as const,
    },
    {
      icon: ListOrdered,
      label: "Ordered list",
      command: "insertOrderedList" as const,
    },
    {
      icon: Quote,
      label: "Quote",
      command: "formatBlock" as const,
      value: "blockquote",
    },
  ];

  return (
    <div
      aria-label="Selection formatting"
      className={cn(
        "absolute z-30 flex items-center gap-0.5 rounded-full p-1",
        ACTION_TRAY_SURFACE_CLASS,
        POPOVER_CUSTOM_ENTER_MOTION_CLASS,
        position.placement === "top"
          ? "-translate-x-1/2 -translate-y-full"
          : "-translate-x-1/2",
      )}
      data-testid="welcome-selection-formatting-tray"
      onMouseDown={(event) => event.preventDefault()}
      role="toolbar"
      style={{
        left: position.left,
        top: position.top,
      }}
    >
      {items.map((item) => (
        <button
          aria-label={item.label}
          className="inline-flex h-7 w-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring [&_svg]:size-4"
          key={item.label}
          onClick={() => {
            if ("action" in item && item.action) {
              item.action();
            } else if ("command" in item) {
              onFormat(item.command, item.value);
            }
          }}
          title={item.label}
          type="button"
        >
          <item.icon />
        </button>
      ))}
    </div>
  );
}

export function WelcomeComposer({
  message,
  onChange,
  onUploadCountChange,
}: {
  message: WelcomeMessage;
  onChange: React.Dispatch<React.SetStateAction<WelcomeMessage>>;
  onUploadCountChange: React.Dispatch<React.SetStateAction<number>>;
}) {
  const channels = useChannelsQuery().data ?? [];
  const wrapperRef = React.useRef<HTMLDivElement>(null);
  const editorRef = React.useRef<HTMLDivElement>(null);
  const savedRangeRef = React.useRef<Range | null>(null);
  const formattingRangeRef = React.useRef<Range | null>(null);
  const [chipEditor, setChipEditor] = React.useState<ChipEditorPosition | null>(
    null,
  );
  const [selectionToolbar, setSelectionToolbar] =
    React.useState<SelectionToolbarPosition | null>(null);
  const [isDraggingImage, setIsDraggingImage] = React.useState(false);
  const [dropCaret, setDropCaret] = React.useState<DropCaretPosition | null>(
    null,
  );

  React.useLayoutEffect(() => {
    const editor = editorRef.current;
    if (!editor || editor.contains(document.activeElement)) return;
    renderEditorContent(editor, message.text, message.inserts);
  }, [message]);

  const rememberCaret = React.useCallback(() => {
    const editor = editorRef.current;
    const selection = document.getSelection();
    if (!editor || !selection?.rangeCount) return;
    const range = selection.getRangeAt(0);
    if (editor.contains(range.commonAncestorContainer)) {
      savedRangeRef.current = range.cloneRange();
    }
  }, []);

  const updateSelectionToolbar = React.useCallback(() => {
    const editor = editorRef.current;
    const wrapper = wrapperRef.current;
    const selection = document.getSelection();
    if (
      !editor ||
      !wrapper ||
      !selection?.rangeCount ||
      selection.isCollapsed
    ) {
      formattingRangeRef.current = null;
      setSelectionToolbar(null);
      return;
    }
    const range = selection.getRangeAt(0);
    if (
      !editor.contains(range.commonAncestorContainer) ||
      range.toString().trim().length === 0
    ) {
      formattingRangeRef.current = null;
      setSelectionToolbar(null);
      return;
    }

    const rect = range.getBoundingClientRect();
    const wrapperRect = wrapper.getBoundingClientRect();
    if (rect.width <= 0 && rect.height <= 0) return;
    formattingRangeRef.current = range.cloneRange();
    const placement = rect.top >= 48 ? "top" : "bottom";
    setSelectionToolbar({
      left: Math.min(
        wrapperRect.width - 156,
        Math.max(156, rect.left + rect.width / 2 - wrapperRect.left),
      ),
      placement,
      top:
        placement === "top"
          ? rect.top - wrapperRect.top - 8
          : rect.bottom - wrapperRect.top + 8,
    });
  }, []);

  function rememberContextInsertionPoint(
    event: React.MouseEvent<HTMLDivElement>,
  ) {
    const editor = editorRef.current;
    if (!editor) return;

    const range = document.caretRangeFromPoint?.(event.clientX, event.clientY);
    if (!range || !editor.contains(range.commonAncestorContainer)) return;

    range.collapse(true);
    savedRangeRef.current = range.cloneRange();
    const selection = document.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  }

  function insertAtCaret(
    type: WelcomeInsertType | "member",
    insertionRange = savedRangeRef.current,
    title?: string,
  ): string | null {
    const editor = editorRef.current;
    const range = insertionRange;
    if (!editor || !range || !editor.contains(range.commonAncestorContainer)) {
      return null;
    }
    let node: HTMLElement;
    let inserts = message.inserts;
    let insertedId: string | null = null;
    if (type === "member") {
      node = makeMemberChip();
    } else {
      const insert: WelcomeInsert = {
        id: crypto.randomUUID(),
        type,
        title:
          title ||
          (type === "link"
            ? "New link"
            : type === "image"
              ? "New image"
              : "Choose a channel"),
        url: "",
      };
      insertedId = insert.id;
      inserts = [...inserts, insert];
      node = makeInsertChip(insert);
    }

    range.deleteContents();
    range.insertNode(node);
    const spacer = document.createTextNode(" ");
    node.after(spacer);
    range.setStartAfter(spacer);
    range.collapse(true);

    const selection = document.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    savedRangeRef.current = range.cloneRange();
    onChange({ text: readEditorContent(editor), inserts });
    setSelectionToolbar(null);
    editor.focus();
    if (type === "channel" && insertedId) {
      window.requestAnimationFrame(() => openChipEditor(node, insertedId));
    }
    return insertedId;
  }

  function openChipEditor(chip: HTMLElement, id: string) {
    const wrapper = wrapperRef.current;
    if (!wrapper) return;
    const chipRect = chip.getBoundingClientRect();
    const wrapperRect = wrapper.getBoundingClientRect();
    setChipEditor({
      id,
      left: Math.min(
        chipRect.left - wrapperRect.left,
        Math.max(0, wrapperRect.width - 320),
      ),
      top: chipRect.bottom - wrapperRect.top + 6,
    });
  }

  function imageDropRangeAtPoint(clientX: number, clientY: number) {
    const editor = editorRef.current;
    const wrapper = wrapperRef.current;
    if (!editor || !wrapper) return null;
    let range = document.caretRangeFromPoint?.(clientX, clientY);
    if (!range || !editor.contains(range.commonAncestorContainer)) return null;

    const rangeElement =
      range.startContainer instanceof Element
        ? range.startContainer
        : range.startContainer.parentElement;
    const chip = rangeElement?.closest<HTMLElement>(
      "[contenteditable='false']",
    );
    if (chip && editor.contains(chip)) {
      const chipRect = chip.getBoundingClientRect();
      range = document.createRange();
      if (clientX < chipRect.left + chipRect.width / 2) {
        range.setStartBefore(chip);
      } else {
        range.setStartAfter(chip);
      }
    }

    range.collapse(true);
    savedRangeRef.current = range.cloneRange();
    const selection = document.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);

    const rangeRect =
      range.getClientRects()[0] ?? range.getBoundingClientRect();
    const wrapperRect = wrapper.getBoundingClientRect();
    const editorRect = editor.getBoundingClientRect();
    const lineHeight = Number.parseFloat(getComputedStyle(editor).lineHeight);
    const height = Math.max(18, rangeRect.height || lineHeight || 24);
    const left = rangeRect.height > 0 ? rangeRect.left : clientX;
    const top =
      rangeRect.height > 0
        ? rangeRect.top
        : Math.min(
            editorRect.bottom - height,
            Math.max(editorRect.top, clientY - height / 2),
          );
    setDropCaret({
      height,
      left: Math.min(
        wrapperRect.width - 2,
        Math.max(0, left - wrapperRect.left),
      ),
      top: Math.min(
        wrapperRect.height - height,
        Math.max(0, top - wrapperRect.top),
      ),
    });
    return range;
  }

  function handleImageDrop(event: React.DragEvent<HTMLDivElement>) {
    const image = Array.from(event.dataTransfer.files).find((file) =>
      file.type.startsWith("image/"),
    );
    if (!image) return;
    event.preventDefault();
    const range =
      imageDropRangeAtPoint(event.clientX, event.clientY) ??
      savedRangeRef.current;
    setIsDraggingImage(false);
    setDropCaret(null);
    const insertId = insertAtCaret("image", range, image.name);
    if (!insertId) return;

    onUploadCountChange((count) => count + 1);
    void uploadMediaFile(image)
      .then((descriptor) => {
        onChange((current) => ({
          ...current,
          inserts: current.inserts.map((insert) =>
            insert.id === insertId
              ? { ...insert, title: image.name, url: descriptor.url }
              : insert,
          ),
        }));
      })
      .catch(() => {
        toast.error("Image couldn’t be uploaded. Add an image link instead.");
      })
      .finally(() => onUploadCountChange((count) => Math.max(0, count - 1)));
  }

  function applyFormatting(
    command:
      | "bold"
      | "formatBlock"
      | "insertOrderedList"
      | "insertUnorderedList"
      | "italic"
      | "strikeThrough",
    value?: string,
  ) {
    const editor = editorRef.current;
    const range = formattingRangeRef.current;
    if (!editor || !range) return;
    const selection = document.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    document.execCommand(command, false, value);
    onChange({ ...message, text: readEditorContent(editor) });
    window.requestAnimationFrame(updateSelectionToolbar);
  }

  function handleEditorClick(event: React.MouseEvent<HTMLDivElement>) {
    const chip = (event.target as HTMLElement).closest<HTMLElement>(
      "[data-insert-id]",
    );
    if (!chip?.dataset.insertId) {
      setChipEditor(null);
      return;
    }
    openChipEditor(chip, chip.dataset.insertId);
  }

  const selectedInsert = chipEditor
    ? message.inserts.find((insert) => insert.id === chipEditor.id)
    : undefined;

  return (
    <div className="relative h-full min-h-64" ref={wrapperRef}>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          {/* biome-ignore lint/a11y/useSemanticElements: contentEditable is required for mixed free text and non-editable inline chips. */}
          <div
            aria-label="Welcome message"
            className={`${MESSAGE_MARKDOWN_CLASS} h-full min-h-64 whitespace-pre-wrap break-words text-message leading-6 text-foreground outline-none empty:before:text-muted-foreground/65 empty:before:content-['Write_a_welcome_message…'] [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:px-2 [&_pre]:py-1 [&_ul]:list-disc [&_ul]:pl-6`}
            contentEditable
            data-testid="welcome-inline-message"
            onClick={handleEditorClick}
            onContextMenu={rememberContextInsertionPoint}
            onDragEnter={(event) => {
              if (
                Array.from(event.dataTransfer.items).some(
                  (item) =>
                    item.kind === "file" && item.type.startsWith("image/"),
                )
              ) {
                setIsDraggingImage(true);
                imageDropRangeAtPoint(event.clientX, event.clientY);
              }
            }}
            onDragLeave={(event) => {
              if (
                !(event.relatedTarget instanceof Node) ||
                !event.currentTarget.contains(event.relatedTarget)
              ) {
                setIsDraggingImage(false);
                setDropCaret(null);
              }
            }}
            onDragOver={(event) => {
              if (
                Array.from(event.dataTransfer.items).some(
                  (item) =>
                    item.kind === "file" && item.type.startsWith("image/"),
                )
              ) {
                event.preventDefault();
                event.dataTransfer.dropEffect = "copy";
                setIsDraggingImage(true);
                imageDropRangeAtPoint(event.clientX, event.clientY);
              }
            }}
            onDrop={handleImageDrop}
            onFocus={rememberCaret}
            onInput={(event) => {
              onChange({
                ...message,
                text: readEditorContent(event.currentTarget),
              });
              rememberCaret();
            }}
            onKeyUp={() => {
              rememberCaret();
              updateSelectionToolbar();
            }}
            onMouseUp={() => {
              rememberCaret();
              updateSelectionToolbar();
            }}
            ref={editorRef}
            role="textbox"
            spellCheck
            suppressContentEditableWarning
            tabIndex={0}
          />
        </ContextMenuTrigger>
        <ContextMenuContent className={ACTION_TRAY_SURFACE_CLASS}>
          <ContextMenuItem
            className="hover:bg-muted/50 hover:text-foreground"
            onSelect={() => insertAtCaret("member")}
          >
            <UserRound className="h-4 w-4" />
            New member’s name
          </ContextMenuItem>
          <ContextMenuItem
            className="hover:bg-muted/50 hover:text-foreground"
            onSelect={() => insertAtCaret("link")}
          >
            <Link2 className="h-4 w-4" />
            Link
          </ContextMenuItem>
          <ContextMenuItem
            className="hover:bg-muted/50 hover:text-foreground"
            onSelect={() => insertAtCaret("image")}
          >
            <ImageIcon className="h-4 w-4" />
            Image
          </ContextMenuItem>
          <ContextMenuItem
            className="hover:bg-muted/50 hover:text-foreground"
            onSelect={() => {
              window.setTimeout(() => insertAtCaret("channel"));
            }}
          >
            <Hash className="h-4 w-4" />
            Channel
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      {isDraggingImage ? (
        <div className="pointer-events-none absolute right-2 top-2 z-20 rounded-full border border-border/70 bg-background/95 px-3 py-1.5 text-xs font-medium text-foreground shadow-xs backdrop-blur-sm">
          Drop image here
        </div>
      ) : null}

      {isDraggingImage && dropCaret ? (
        <div
          aria-hidden
          className="pointer-events-none absolute z-20 w-0.5 rounded-full bg-foreground"
          data-testid="welcome-image-drop-caret"
          style={{
            height: dropCaret.height,
            left: dropCaret.left - 1,
            top: dropCaret.top,
          }}
        />
      ) : null}

      {selectionToolbar ? (
        <WelcomeSelectionToolbar
          onFormat={applyFormatting}
          onLink={() => {
            const range = formattingRangeRef.current;
            if (!range) return;
            insertAtCaret("link", range, range.toString().trim());
          }}
          position={selectionToolbar}
        />
      ) : null}

      {selectedInsert && chipEditor ? (
        <InlineChipEditor
          channels={channels}
          insert={selectedInsert}
          onChange={(nextInsert) =>
            onChange({
              ...message,
              inserts: message.inserts.map((insert) =>
                insert.id === nextInsert.id ? nextInsert : insert,
              ),
            })
          }
          onClose={() => setChipEditor(null)}
          onRemove={() => {
            onChange({
              text: message.text.replace(insertToken(selectedInsert.id), ""),
              inserts: message.inserts.filter(
                (insert) => insert.id !== selectedInsert.id,
              ),
            });
            setChipEditor(null);
          }}
          position={chipEditor}
        />
      ) : null}
    </div>
  );
}

export function WelcomePreview({ message }: { message: WelcomeMessage }) {
  const previewRef = React.useRef<HTMLDivElement>(null);
  const mediaProxyPort = useMediaProxyPort();

  React.useLayoutEffect(() => {
    const preview = previewRef.current;
    if (!preview) return;
    renderEditorContent(preview, message.text, message.inserts, {
      interactive: false,
      mediaProxyPort,
      memberLabel: "Alex",
      renderImages: true,
    });
  }, [mediaProxyPort, message]);

  return (
    <div
      className={`${MESSAGE_MARKDOWN_CLASS} min-h-64 flex-1 whitespace-pre-wrap rounded-2xl border border-border/50 bg-background/80 px-4 py-3 text-message leading-6 [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:px-2 [&_pre]:py-1 [&_ul]:list-disc [&_ul]:pl-6`}
      data-testid="welcome-channel-preview"
      ref={previewRef}
    />
  );
}
