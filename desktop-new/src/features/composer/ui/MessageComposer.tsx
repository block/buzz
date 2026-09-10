import { ChipNode } from "../chipNode";
import { Select } from "@/shared/ui/Select";
import {
  IconArrowUp,
  IconLetterCase,
  IconMicrophone,
  IconPaperclip,
  IconPlayerStopFilled,
} from "@tabler/icons-react";
import { EditorContent, useEditor } from "@tiptap/react";
import Link from "@tiptap/extension-link";
import { Extension } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown as TiptapMarkdown } from "tiptap-markdown";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";

import { IconButton } from "@/shared/ui/IconButton";
import {
  ComposerFormattingBar,
  type ComposerFormat,
} from "./ComposerFormattingBar";
import { ComposerLinkDialog } from "./ComposerLinkDialog";
import { EmojiPicker } from "./EmojiPicker";

const FORMATS: readonly ComposerFormat[] = [
  "bold",
  "italic",
  "strike",
  "code",
  "quote",
  "bullet",
  "number",
];

function markdown(editor: NonNullable<ReturnType<typeof useEditor>>) {
  // tiptap-markdown has no public TypeScript storage shape.
  const storage = editor.storage as unknown as {
    markdown?: { getMarkdown?: () => string };
  };
  const value = storage.markdown?.getMarkdown?.() ?? editor.getText();
  return value.replace(/\\\n/g, "\n");
}

/** The first production Composer composition, with Markdown-compatible formatting. */
export function MessageComposer({
  disabled,
  draft,
  onDraftChange,
  onSend,
  placeholder = "Message the session",
  responseControl,
  autoFocus = true,
  defaultFormattingOpen = false,
  recipients = [],
  onDelivered,
}: {
  onDelivered?: (content: string) => void;
  recipients?: readonly { pubkey: string; name: string; isAgent: boolean }[];
  autoFocus?: boolean;
  defaultFormattingOpen?: boolean;
  disabled?: boolean;
  draft: string;
  onDraftChange: (value: string) => void;
  onSend: (content: string, recipients?: string[]) => Promise<void>;
  placeholder?: string;
  responseControl?: { state: "responding" | "stopping"; onStop?: () => void };
}) {
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [formattingOpen, setFormattingOpen] = useState(defaultFormattingOpen);
  const [formattingLeaving, setFormattingLeaving] = useState(false);
  const [linkOpen, setLinkOpen] = useState(false);
  const [, setEditorVersion] = useState(0);
  const errorId = useId();
  const applyingContent = useRef(false);
  const submitting = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const formattingSelection = useRef<{ from: number; to: number } | null>(null);
  const latest = useRef({ draft, onDraftChange, onSend, disabled, sending });
  const submitRef = useRef<() => void>(() => {});
  latest.current = { draft, onDraftChange, onSend, disabled, sending };

  const editor = useEditor({
    extensions: [
      ChipNode,
      StarterKit.configure({
        hardBreak: { keepMarks: true },
        heading: false,
        link: false,
        trailingNode: false,
      }),
      Extension.create({
        name: "submitOnEnter",
        addKeyboardShortcuts() {
          return {
            Enter: () => {
              submitRef.current();
              return true;
            },
          };
        },
      }),
      Link.configure({
        openOnClick: false,
        autolink: true,
        linkOnPaste: false,
      }),
      Placeholder.configure({ placeholder }),
      TiptapMarkdown.configure({
        html: false,
        breaks: true,
        transformCopiedText: true,
        transformPastedText: true,
      }),
    ],
    content: draft,
    editable: !disabled,
    onCreate: ({ editor: current }) => {
      current.view.dom.setAttribute("role", "textbox");
      current.view.dom.setAttribute("aria-multiline", "true");
    },
    editorProps: {
      attributes: {
        class: "message-composer-editor",
        "aria-label": placeholder,
      },
    },
    onUpdate: ({ editor: current }) => {
      if (applyingContent.current) return;
      setError(null);
      latest.current.onDraftChange(markdown(current));
    },
  });

  useEffect(() => {
    if (!editor) return;
    const update = () => setEditorVersion((version) => version + 1);
    editor.on("transaction", update);
    return () => {
      editor.off("transaction", update);
    };
  }, [editor]);
  useEffect(() => {
    if (autoFocus) editor?.commands.focus("end");
  }, [autoFocus, editor]);
  useEffect(() => {
    editor?.setEditable(!disabled);
  }, [disabled, editor]);
  useEffect(() => {
    if (!editor || markdown(editor) === draft) return;
    applyingContent.current = true;
    editor.commands.setContent(draft, { emitUpdate: false });
    applyingContent.current = false;
  }, [draft, editor]);

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    const content = editor ? markdown(editor).trim() : draft.trim();
    if (!content || submitting.current || disabled) return;
    submitting.current = true;
    setSending(true);
    setError(null);
    try {
      const selectedKeys: string[] = [];
      editor?.state.doc.descendants((node) => {
        if (
          node.type.name === "chip" &&
          ["agent", "person"].includes(node.attrs.kind)
        )
          selectedKeys.push(node.attrs.id);
      });
      await onSend(content, selectedKeys);
      onDelivered?.(content);
      if (mounted.current && latest.current.draft.trim() === content) {
        if (!onDelivered) onDraftChange("");
        if (mounted.current) editor?.commands.clearContent(true);
      }
    } catch (caught) {
      if (mounted.current)
        setError(
          caught instanceof Error
            ? caught.message
            : "Your message was not sent.",
        );
    } finally {
      submitting.current = false;
      if (mounted.current) {
        setSending(false);
        editor?.commands.focus("end");
      }
    }
  }

  submitRef.current = () => {
    void submit();
  };

  function closeFormatting() {
    setFormattingLeaving(true);
  }

  function insertEmoji(emoji: string) {
    editor?.chain().focus().insertContent(emoji).run();
  }
  function rememberFormattingSelection() {
    if (!editor || editor.state.selection.empty) return;
    formattingSelection.current = {
      from: editor.state.selection.from,
      to: editor.state.selection.to,
    };
  }

  function format(format: ComposerFormat) {
    if (format === "link") {
      setLinkOpen(true);
      return;
    }
    const chain = editor?.chain();
    if (!chain) return;
    const range = formattingSelection.current;
    formattingSelection.current = null;
    if (range) chain.setTextSelection(range);
    if (format === "bold") chain.toggleBold().run();
    if (format === "italic") chain.toggleItalic().run();
    if (format === "strike") chain.toggleStrike().run();
    if (format === "code") chain.toggleCode().run();
    if (format === "quote") chain.toggleBlockquote().run();
    if (format === "bullet") chain.toggleBulletList().run();
    if (format === "number") chain.toggleOrderedList().run();
  }
  function insertLink({ label, url }: { label: string; url: string }) {
    editor
      ?.chain()
      .focus()
      .extendMarkRange("link")
      .setLink({ href: url })
      .insertContent(label)
      .run();
  }
  const unavailable = Boolean(disabled || sending);
  const canSend = Boolean(draft.trim()) && !unavailable;
  const state = disabled
    ? "disabled"
    : sending
      ? "sending"
      : error
        ? "failed"
        : "ready";
  const activeFormats = FORMATS.filter((format) => {
    const names: Record<Exclude<ComposerFormat, "link">, string> = {
      bold: "bold",
      italic: "italic",
      strike: "strike",
      code: "code",
      quote: "blockquote",
      bullet: "bulletList",
      number: "orderedList",
    };
    return format !== "link" && editor?.isActive(names[format]);
  });

  return (
    <>
      <form className="message-composer" data-state={state} onSubmit={submit}>
        {recipients.length ? (
          <Select
            label="Address"
            value=""
            onValueChange={(pubkey) => {
              const person = recipients.find(
                (candidate) => candidate.pubkey === pubkey,
              );
              if (!person || unavailable) return;
              editor
                ?.chain()
                .focus()
                .insertContent([
                  {
                    type: "chip",
                    attrs: {
                      kind: person.isAgent ? "agent" : "person",
                      id: pubkey,
                    },
                  },
                  { type: "text", text: " " },
                ])
                .run();
            }}
            groups={[
              {
                label: "People and agents in this Session",
                options: recipients.map((person) => ({
                  value: person.pubkey,
                  label: person.name,
                })),
              },
            ]}
          />
        ) : null}
        <div className="message-composer-input">
          <EditorContent editor={editor} />
          {error ? (
            <p
              id={errorId}
              className="message-composer-feedback text-body-sm text-red-12"
              role="alert"
            >
              {error} Your message is still here — try again.
            </p>
          ) : sending ? (
            <p
              className="message-composer-feedback text-body-sm text-secondary"
              role="status"
            >
              Sending message…
            </p>
          ) : null}
        </div>
        <div className="message-composer-actions">
          <div className="composer-toolbar-presence">
            {formattingOpen ? (
              <ComposerFormattingBar
                activeFormats={activeFormats}
                onCommandPointerDown={rememberFormattingSelection}
                disabled={unavailable}
                onFormat={format}
                onClose={closeFormatting}
                className={
                  formattingLeaving
                    ? "composer-toolbar-exit"
                    : "composer-toolbar-enter"
                }
                onAnimationEnd={() => {
                  if (!formattingLeaving) return;
                  setFormattingOpen(false);
                  setFormattingLeaving(false);
                  editor?.commands.focus();
                }}
              />
            ) : (
              <div className="message-composer-actions-start composer-toolbar-enter">
                <IconButton
                  aria-label="Attach file"
                  disabled={unavailable}
                  icon={
                    <IconPaperclip size={16} stroke={2} aria-hidden="true" />
                  }
                  size="toolbar"
                  variant="ghost"
                />
                <EmojiPicker disabled={unavailable} onInsert={insertEmoji} />
                <IconButton
                  aria-label="Toggle formatting"
                  aria-expanded={formattingOpen}
                  disabled={unavailable}
                  onMouseDown={rememberFormattingSelection}
                  onClick={() => {
                    setFormattingLeaving(false);
                    setFormattingOpen(true);
                  }}
                  icon={
                    <IconLetterCase size={16} stroke={2} aria-hidden="true" />
                  }
                  size="toolbar"
                  variant="ghost"
                />
              </div>
            )}
          </div>
          <div className="message-composer-actions-end">
            <IconButton
              aria-label="Record voice note"
              disabled={unavailable}
              icon={<IconMicrophone size={16} stroke={2} aria-hidden="true" />}
              size="toolbar"
              variant="ghost"
            />
            {responseControl ? (
              <IconButton
                aria-label={
                  responseControl.state === "stopping"
                    ? "Stopping response"
                    : "Stop response"
                }
                aria-busy={responseControl.state === "stopping" || undefined}
                icon={<IconPlayerStopFilled size={14} aria-hidden="true" />}
                shape="round"
                size="toolbar"
                variant="tint"
                disabled={responseControl.state === "stopping"}
                onClick={responseControl.onStop}
              />
            ) : (
              <IconButton
                type="submit"
                aria-label={sending ? "Sending message" : "Send message"}
                icon={<IconArrowUp size={16} stroke={2} aria-hidden="true" />}
                shape="round"
                size="toolbar"
                variant="tint"
                disabled={!canSend}
              />
            )}
          </div>
        </div>
      </form>
      <ComposerLinkDialog
        open={linkOpen}
        onOpenChange={setLinkOpen}
        onSubmit={insertLink}
      />
    </>
  );
}
