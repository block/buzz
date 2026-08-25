export type DecimalValue = string | number | null | undefined;

function numeric(value: DecimalValue) {
  if (value === null || value === undefined || value === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function formatDecimal(value: DecimalValue, fallback = "—") {
  const parsed = numeric(value);
  return parsed === null
    ? fallback
    : parsed.toLocaleString("zh-CN", {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      });
}

export const formatAmount = formatDecimal;
export const formatQuantity = formatDecimal;

export function formatMoney(currency: string, value: DecimalValue) {
  return `${currency} ${formatAmount(value)}`;
}

export function formatSignedQuantity(value: DecimalValue) {
  const parsed = numeric(value);
  if (parsed === null) return "—";
  return `${parsed > 0 ? "+" : ""}${formatQuantity(parsed)}`;
}

export function fixedDecimal(value: DecimalValue, fallback = "0.00") {
  const parsed = numeric(value);
  return parsed === null ? fallback : parsed.toFixed(2);
}
