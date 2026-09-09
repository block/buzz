import { Popover as BasePopover } from "@base-ui/react/popover";
import data from "@emoji-mart/data";
import {
  IconMoodSmile,
  IconPhoto,
  IconSearch,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { IconButton } from "@/shared/ui/IconButton";
import { Tabs } from "@/shared/ui/Tabs";

type PickerTab = "emoji" | "gifs";
type CustomEmoji = { name: string; src: string };
const CUSTOM: readonly CustomEmoji[] = [
  {
    name: "party-parrot",
    src: "https://em-content.zobj.net/source/twitter/376/parrot_1f99c.png",
  },
  {
    name: "buzz",
    src: "https://em-content.zobj.net/source/twitter/376/honeybee_1f41d.png",
  },
  {
    name: "ship-it",
    src: "https://em-content.zobj.net/source/twitter/376/rocket_1f680.png",
  },
  {
    name: "coffee",
    src: "https://em-content.zobj.net/source/twitter/376/hot-beverage_2615.png",
  },
];
const GIFS = [
  "https://media.giphy.com/media/v1.Y2lkPTc5MGI3NjExNHoyNGl0Z3Q4a3Jkdm1teW93MjVqM2VtY2E2dTQxdmNxdG55dW1nZzVpeSZlcD12MV9naWZzX3NlYXJjaCZjdD1n/3oKIPwoeGErMmaI43S/giphy.gif",
  "https://media.giphy.com/media/v1.Y2lkPTc5MGI3NjExNHoyNGl0Z3Q4a3Jkdm1teW93MjVqM2VtY2E2dTQxdmNxdG55dW1nZzVpeSZlcD12MV9naWZzX3NlYXJjaCZjdD1n/l0MYt5jPR6QX5pnqM/giphy.gif",
];
const CATEGORIES = [
  { id: "frequent", label: "Frequently used", icon: "＋" },
  { id: "smileys", label: "Smileys & people", icon: "☺" },
  { id: "animals", label: "Animals & nature", icon: "♞" },
  { id: "food", label: "Food & drink", icon: "◉" },
  { id: "travel", label: "Travel & places", icon: "⌂" },
  { id: "objects", label: "Objects", icon: "♢" },
  { id: "symbols", label: "Symbols", icon: "＊" },
] as const;
type EmojiCategory = (typeof CATEGORIES)[number]["id"];
const FREQUENT = [
  "👍",
  "🎉",
  "👀",
  "🙏",
  "💜",
  "🔥",
  "✅",
  "🚀",
  "💡",
  "😂",
  "😊",
  "🤔",
];

/** UI-only composer picker: data is seeded until community and GIF capabilities arrive. */
export function EmojiPicker({
  disabled,
  onInsert,
}: {
  disabled?: boolean;
  onInsert: (emoji: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<PickerTab>("emoji");
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<EmojiCategory>("frequent");
  const [preview, setPreview] = useState<{
    emoji: string;
    name: string;
    src?: string;
  } | null>(null);
  const previewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(
    () => () => {
      if (previewTimer.current) clearTimeout(previewTimer.current);
    },
    [],
  );
  function previewAfterPause(value: {
    emoji: string;
    name: string;
    src?: string;
  }) {
    if (previewTimer.current) clearTimeout(previewTimer.current);
    previewTimer.current = setTimeout(() => setPreview(value), 450);
  }
  function clearPreview() {
    if (previewTimer.current) clearTimeout(previewTimer.current);
    previewTimer.current = null;
    setPreview(null);
  }
  const matches = useMemo(() => {
    const term = query.trim().toLowerCase();
    if (!term && category === "frequent")
      return FREQUENT.map((emoji) => ({ emoji, name: "Emoji name" }));
    const emojis = (
      data as unknown as {
        emojis: Record<
          string,
          { name: string; skins?: { native: string }[]; keywords?: string[] }
        >;
      }
    ).emojis;
    return Object.values(emojis)
      .filter((emoji) => {
        const keywords = [emoji.name, ...(emoji.keywords ?? [])];
        return term ? keywords.some((value) => value.includes(term)) : true;
      })
      .slice(0, 42)
      .map((emoji) => ({
        emoji: emoji.skins?.[0]?.native ?? "",
        name: emoji.name,
      }))
      .filter((emoji) => Boolean(emoji.emoji));
  }, [category, query]);
  const custom = CUSTOM.filter((emoji) =>
    emoji.name.includes(query.trim().toLowerCase()),
  );
  const standard = matches;
  function choose(value: string) {
    onInsert(value);
    setOpen(false);
    setQuery("");
  }
  return (
    <BasePopover.Root
      modal={false}
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setQuery("");
      }}
    >
      <BasePopover.Trigger
        render={
          <IconButton
            aria-label="Choose emoji or GIF"
            disabled={disabled}
            icon={<IconMoodSmile size={16} stroke={2} aria-hidden="true" />}
            size="toolbar"
            variant="ghost"
            onMouseDown={(event) => event.preventDefault()}
          />
        }
      />
      <BasePopover.Portal>
        <BasePopover.Positioner side="top" align="start" sideOffset={8}>
          <BasePopover.Popup
            className="composer-expression-picker"
            initialFocus={false}
          >
            <Tabs
              label="Expression type"
              value={tab}
              onValueChange={setTab}
              variant="panel"
              items={[
                {
                  value: "emoji",
                  label: "Emoji",
                  icon: <IconMoodSmile size={16} />,
                },
                { value: "gifs", label: "GIFs", icon: <IconPhoto size={16} /> },
              ]}
            />
            {tab === "emoji" ? (
              <div className="composer-emoji-content">
                <label className="composer-expression-search">
                  <IconSearch size={16} aria-hidden="true" />
                  <input
                    ref={input}
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder="Search emoji"
                    aria-label="Search emoji"
                  />
                  <IconButton
                    aria-label="Clear emoji search"
                    icon={<IconX size={14} aria-hidden="true" />}
                    size="compact"
                    variant="ghost"
                    disabled={!query}
                    onClick={() => {
                      setQuery("");
                      input.current?.focus();
                    }}
                  />
                </label>
                {!query ? (
                  <div
                    className="composer-emoji-categories"
                    role="tablist"
                    aria-label="Emoji categories"
                  >
                    {CATEGORIES.map((item) => (
                      <button
                        key={item.id}
                        role="tab"
                        aria-selected={category === item.id}
                        aria-label={item.label}
                        type="button"
                        onClick={() => setCategory(item.id)}
                      >
                        {item.icon}
                      </button>
                    ))}
                  </div>
                ) : null}
                {custom.length ? (
                  <section>
                    <h3 className="text-body-sm text-tertiary">Custom</h3>
                    <div className="composer-emoji-grid">
                      {custom.map((emoji) => (
                        <button
                          key={emoji.name}
                          type="button"
                          className="composer-emoji-choice"
                          aria-label={`Insert :${emoji.name}:`}
                          onClick={() => choose(`:${emoji.name}:`)}
                          onPointerEnter={() =>
                            previewAfterPause({
                              emoji: `:${emoji.name}:`,
                              name: emoji.name,
                              src: emoji.src,
                            })
                          }
                          onPointerLeave={clearPreview}
                          onFocus={() =>
                            previewAfterPause({
                              emoji: `:${emoji.name}:`,
                              name: emoji.name,
                              src: emoji.src,
                            })
                          }
                          onBlur={clearPreview}
                        >
                          <img src={emoji.src} alt="" />
                        </button>
                      ))}
                    </div>
                  </section>
                ) : null}
                <section>
                  <h3 className="text-body-sm text-tertiary">
                    {query
                      ? "Results"
                      : CATEGORIES.find((item) => item.id === category)?.label}
                  </h3>
                  <div className="composer-emoji-grid">
                    {standard.map(({ emoji, name }) => (
                      <button
                        key={emoji}
                        type="button"
                        className="composer-emoji-choice"
                        aria-label={`Insert ${emoji}`}
                        onClick={() => choose(emoji)}
                        onPointerEnter={() =>
                          previewAfterPause({ emoji, name })
                        }
                        onPointerLeave={clearPreview}
                        onFocus={() => previewAfterPause({ emoji, name })}
                        onBlur={clearPreview}
                      >
                        {emoji}
                      </button>
                    ))}
                  </div>
                </section>
                <div className="composer-emoji-preview" aria-live="polite">
                  {preview ? (
                    <>
                      {preview.src ? (
                        <img src={preview.src} alt="" />
                      ) : (
                        <span
                          className="composer-emoji-preview-glyph"
                          aria-hidden="true"
                        >
                          {preview.emoji}
                        </span>
                      )}
                      <div>
                        <strong className="text-body-lg text-primary">
                          {preview.name}
                        </strong>
                      </div>
                    </>
                  ) : null}
                </div>
              </div>
            ) : (
              <div className="composer-gif-content">
                <label className="composer-expression-search">
                  <IconSearch size={16} aria-hidden="true" />
                  <input
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder="Search GIFs"
                    aria-label="Search GIFs"
                  />
                </label>
                <div className="composer-gif-grid">
                  {GIFS.map((src, index) => (
                    <button
                      key={src}
                      type="button"
                      aria-label={`Choose GIF ${index + 1}`}
                      onClick={() => setOpen(false)}
                    >
                      <img src={src} alt="" />
                    </button>
                  ))}
                </div>
                <p className="text-body-sm text-tertiary">
                  GIF search is a design preview.
                </p>
              </div>
            )}
          </BasePopover.Popup>
        </BasePopover.Positioner>
      </BasePopover.Portal>
    </BasePopover.Root>
  );
}
