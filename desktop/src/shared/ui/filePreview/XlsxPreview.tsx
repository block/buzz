import * as React from "react";
import * as XLSX from "xlsx";

import { cn } from "@/shared/lib/cn";

/**
 * Renders a .xlsx workbook as an HTML table via SheetJS, with a tab switcher
 * when the workbook has more than one sheet.
 *
 * `sheet_to_html` escapes cell values into table markup (it renders data, not
 * arbitrary source HTML), so this doesn't carry the same "attacker-controlled
 * href/src" caveat DocxPreview has — cell formulas/values become plain text.
 */
export function XlsxPreview({ bytes }: { bytes: Uint8Array }) {
  const [error, setError] = React.useState<string | null>(null);
  const [sheetNames, setSheetNames] = React.useState<string[]>([]);
  const [activeSheet, setActiveSheet] = React.useState<string | null>(null);
  const workbookRef = React.useRef<XLSX.WorkBook | null>(null);

  React.useEffect(() => {
    try {
      const workbook = XLSX.read(bytes, { type: "array" });
      workbookRef.current = workbook;
      setSheetNames(workbook.SheetNames);
      setActiveSheet(workbook.SheetNames[0] ?? null);
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Failed to read spreadsheet",
      );
    }
  }, [bytes]);

  const html = React.useMemo(() => {
    const workbook = workbookRef.current;
    if (!workbook || !activeSheet) return null;
    const sheet = workbook.Sheets[activeSheet];
    if (!sheet) return null;
    return XLSX.utils.sheet_to_html(sheet);
  }, [activeSheet]);

  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center text-sm text-muted-foreground">
        {error}
      </div>
    );
  }

  if (html === null) {
    return (
      <div className="flex justify-center py-8 text-sm text-muted-foreground">
        Loading spreadsheet…
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {sheetNames.length > 1 ? (
        <div className="flex shrink-0 gap-1 overflow-x-auto border-b border-border/70 px-3 py-2">
          {sheetNames.map((name) => (
            <button
              key={name}
              type="button"
              onClick={() => setActiveSheet(name)}
              className={cn(
                "shrink-0 rounded-full px-3 py-1 text-xs font-medium transition-colors",
                name === activeSheet
                  ? "bg-primary text-primary-foreground"
                  : "bg-muted/50 text-muted-foreground hover:bg-muted",
              )}
            >
              {name}
            </button>
          ))}
        </div>
      ) : null}
      <div
        className="min-h-0 flex-1 overflow-auto p-4 text-sm [&_table]:border-collapse [&_td]:border [&_td]:border-border/50 [&_td]:px-2 [&_td]:py-1 [&_th]:border [&_th]:border-border/50 [&_th]:bg-muted/40 [&_th]:px-2 [&_th]:py-1"
        // biome-ignore lint/security/noDangerouslySetInnerHtml: SheetJS sheet_to_html escapes cell values into table markup — this renders spreadsheet data, not source HTML
        dangerouslySetInnerHTML={{ __html: html }}
      />
    </div>
  );
}
