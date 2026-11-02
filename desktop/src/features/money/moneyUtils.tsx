import type * as React from "react";

import {
  BusinessRecordParseError,
  type EventRecord,
} from "@/features/clients/lib/businessRecords";
import {
  formatMoneyMinor,
  type InvoiceLine,
  type MoneyInvoiceHead,
} from "./lib/moneyRecords";

export type InvoiceLineDraft = Omit<
  InvoiceLine,
  "quantityHundredths" | "unitAmountMinor"
> & {
  draftKey: string;
  quantityInput: string;
  amountInput: string;
};

export function filterInvoiceRecords(
  invoices: EventRecord<MoneyInvoiceHead>[],
  period: string,
  clientId: string,
) {
  return invoices.filter(
    ({ event, value }) =>
      (clientId === "all" || value.clientId === clientId) &&
      periodMatches(period, value.issuedAt ?? event.created_at),
  );
}

export function formatCurrencyTotals(
  amounts: Record<string, number>,
): React.ReactNode {
  const entries = Object.entries(amounts)
    .filter(([, amount]) => amount !== 0)
    .sort(([left], [right]) => left.localeCompare(right));
  if (!entries.length) return "-";
  return (
    <span className="b3-currency-totals">
      {entries.map(([currency, amount]) => (
        <span key={currency}>{formatMoneyMinor(amount, currency)}</span>
      ))}
    </span>
  );
}

export function lineTotalMinor(line: InvoiceLine) {
  const total =
    (BigInt(line.quantityHundredths) * BigInt(line.unitAmountMinor)) / 100n;
  if (total > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new BusinessRecordParseError(
      "invoice line amount exceeds the safe display range",
    );
  }
  return Number(total);
}

export function totalLinesMinor(lines: readonly InvoiceLine[]) {
  const total = lines.reduce(
    (sum, line) => sum + BigInt(lineTotalMinor(line)),
    0n,
  );
  if (total > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error("Invoice total exceeds the supported display range.");
  }
  return Number(total);
}

export function updateLine(
  setLines: React.Dispatch<React.SetStateAction<InvoiceLineDraft[]>>,
  index: number,
  patch: Partial<InvoiceLineDraft>,
) {
  setLines((current) =>
    current.map((line, currentIndex) =>
      currentIndex === index ? { ...line, ...patch } : line,
    ),
  );
}

export function invoiceLineDraft(
  line: InvoiceLine,
  currency: string,
): InvoiceLineDraft {
  return {
    draftKey: crypto.randomUUID(),
    serviceId: line.serviceId,
    description: line.description,
    quantityInput: quantityInput(line.quantityHundredths),
    amountInput: decimalInput(line.unitAmountMinor, currency),
  };
}

export function invoiceLineRenderKey(
  line: InvoiceLine,
  lines: InvoiceLine[],
  index: number,
) {
  const canonical = JSON.stringify([
    line.serviceId,
    line.description,
    line.quantityHundredths,
    line.unitAmountMinor,
  ]);
  const duplicateOrdinal = lines
    .slice(0, index)
    .filter(
      (candidate) =>
        JSON.stringify([
          candidate.serviceId,
          candidate.description,
          candidate.quantityHundredths,
          candidate.unitAmountMinor,
        ]) === canonical,
    ).length;
  return `${canonical}:${duplicateOrdinal}`;
}

export function quantityInput(quantityHundredths: number) {
  const whole = Math.floor(quantityHundredths / 100);
  const remainder = quantityHundredths % 100;
  if (remainder === 0) return whole.toString();
  return `${whole}.${remainder.toString().padStart(2, "0").replace(/0+$/, "")}`;
}

export function parseQuantityInput(value: string) {
  const match = /^(\d+)(?:\.(\d{1,2}))?$/.exec(value.trim());
  if (!match) {
    throw new Error("Enter a positive quantity with up to two decimal places.");
  }
  const hundredths =
    BigInt(match[1]) * 100n + BigInt((match[2] ?? "").padEnd(2, "0") || "0");
  if (hundredths <= 0n || hundredths > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error("Enter a positive quantity with up to two decimal places.");
  }
  return Number(hundredths);
}

export function decimalInput(amountMinor: number, currency: string) {
  const digits =
    new Intl.NumberFormat("en-ZA", {
      style: "currency",
      currency,
    }).resolvedOptions().maximumFractionDigits ?? 2;
  const scale = 10n ** BigInt(digits);
  const absolute = BigInt(amountMinor);
  const whole = absolute / scale;
  if (digits === 0) return whole.toString();
  const remainder = absolute % scale;
  const fraction = remainder
    .toString()
    .padStart(digits, "0")
    .replace(/0+$/, "");
  if (!fraction) return whole.toString();
  return `${whole}.${fraction}`;
}

export function moneyStep(currency: string) {
  const digits =
    new Intl.NumberFormat("en-ZA", {
      style: "currency",
      currency,
    }).resolvedOptions().maximumFractionDigits ?? 2;
  return digits === 0 ? "1" : `0.${"0".repeat(digits - 1)}1`;
}

export function currentPeriod() {
  const now = new Date();
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;
}

export function periodForTimestamp(timestamp: number) {
  const date = new Date(timestamp * 1_000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;
}

/** A period is one month ("2026-09") or a span of months ("2026-08..2026-09"). */
export function periodMatches(period: string, timestamp: number) {
  const [from, to = from] = period.split("..");
  const month = periodForTimestamp(timestamp);
  return month >= from && month <= to;
}

export function periodRange(from: string, to: string) {
  return `${from}..${to}`;
}

export function isPeriodRange(period: string) {
  return period.includes("..");
}

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

export function formatPeriod(period: string) {
  if (isPeriodRange(period)) {
    const [from, to] = period.split("..");
    const [fromYear, fromMonth] = from.split("-").map(Number);
    const [toYear, toMonth] = to.split("-").map(Number);
    const start =
      fromYear === toYear
        ? MONTHS[fromMonth - 1]
        : `${MONTHS[fromMonth - 1]} ${fromYear}`;
    return `${start} – ${MONTHS[toMonth - 1]} ${toYear}`;
  }
  const [year, month] = period.split("-").map(Number);
  return new Intl.DateTimeFormat("en-ZA", {
    month: "long",
    year: "numeric",
  }).format(new Date(year, month - 1, 1));
}

/** "20 Sep 2026", or a dash when there is no date. */
export function formatDisplayDate(timestamp: number | null) {
  if (timestamp === null) return "-";
  const date = new Date(timestamp * 1_000);
  return `${date.getDate()} ${MONTHS[date.getMonth()]} ${date.getFullYear()}`;
}

/** Initials derived from the client's display name. */
export function clientInitials(name: string) {
  const words = name
    .trim()
    .split(/\s+/)
    .filter((word, index) => index > 0 || word.toLowerCase() !== "the");
  return words
    .slice(0, 2)
    .map((word) => word.charAt(0).toUpperCase())
    .join("");
}

/** The short reference shown as "Invoice 0027": the end of the invoice id. */
export function invoiceNumber(invoiceId: string) {
  return invoiceId.slice(-4).toUpperCase();
}

export type InvoiceChip =
  | "draft"
  | "awaiting"
  | "part"
  | "overdue"
  | "paid"
  | "void";

export const INVOICE_CHIP_LABELS: Record<InvoiceChip, string> = {
  draft: "Draft",
  awaiting: "Awaiting payment",
  part: "Part paid",
  overdue: "Overdue",
  paid: "Paid",
  void: "Void",
};

/** Every status an invoice shows: a part paid invoice can also be overdue. */
export function invoiceChips(invoice: MoneyInvoiceHead): InvoiceChip[] {
  if (invoice.status === "void") return ["void"];
  if (invoice.status === "draft") return ["draft"];
  if (invoice.outstandingMinor === 0) return ["paid"];
  const chips: InvoiceChip[] = [
    invoice.collectedMinor > 0 ? "part" : "awaiting",
  ];
  if (isOverdue(invoice)) chips.push("overdue");
  return chips;
}

export function formatTimestamp(timestamp: number | null) {
  if (timestamp === null) return "-";
  const date = new Date(timestamp * 1_000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export function dateInput(timestamp: number | null) {
  if (timestamp === null) return "";
  const date = new Date(timestamp * 1_000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export function timestampFromDateInput(value: string): number | null {
  if (!value) return null;
  const timestamp = new Date(`${value}T12:00:00`).getTime() / 1_000;
  if (!Number.isSafeInteger(timestamp) || timestamp < 0)
    throw new Error("Choose a valid date.");
  return timestamp;
}

export function currentDate() {
  const now = new Date();
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
}

export function isOverdue(invoice: MoneyInvoiceHead) {
  return (
    invoice.status === "issued" &&
    invoice.outstandingMinor > 0 &&
    invoice.dueAt !== null &&
    invoice.dueAt < Math.floor(Date.now() / 1_000)
  );
}

export function displayError(error: unknown) {
  if (error instanceof Error && error.message.trim()) return error.message;
  return "The money record could not be saved. Your entered values are still here. Retry when the connection is available.";
}

export function navigateMoney(path: string) {
  window.location.hash = `#${path}`;
}
