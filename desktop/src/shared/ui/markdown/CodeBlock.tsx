import * as React from "react";
import { Copy } from "lucide-react";
import { toast } from "sonner";
import {
  getSingletonHighlighter,
  type HighlighterGeneric,
  type BundledLanguage,
  type BundledTheme,
  type ThemedToken,
} from "shiki";

import { useTheme } from "@/shared/theme/ThemeProvider";
import { copyCodeBlockToClipboard } from "@/shared/lib/codeBlockClipboard";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import { getReactNodeText } from "./utils";

let shikiHighlighter: HighlighterGeneric<BundledLanguage, BundledTheme> | null =
  null;
let shikiInitPromise: Promise<void> | null = null;
const loadedLangs = new Set<string>();
const loadedThemes = new Set<string>();
const tokenCache = new Map<string, ThemedToken[][]>();
const MAX_CACHE_ENTRIES = 100;
const MAX_LOADED_LANGUAGES = 30;
const MAX_HIGHLIGHT_LINES = 150;
export const CODE_BLOCK_CLASS =
  "code-block-lines block min-w-full whitespace-pre font-mono text-sm font-medium text-foreground";
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

export function extractLanguage(className?: string): string {
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

function getCodeBlockText(children: React.ReactNode) {
  return getReactNodeText(children).replace(/\n$/, "");
}

export function MarkdownCodeBlock({
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
            className="absolute right-2 top-2 z-20 h-7 w-7 bg-background/80 text-muted-foreground opacity-0 shadow-xs ring-1 ring-border/60 backdrop-blur-sm transition-opacity hover:bg-background hover:text-foreground hover:opacity-100 focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 disabled:opacity-60"
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

export function SyntaxHighlightedCode({
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
