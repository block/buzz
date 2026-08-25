import React from "react";
import { type ReturnOptionLine, type ReturnOptions, request } from "./api";
import { formatQuantity } from "./formatters";
import "./return-entry.css";

type Side = "sales" | "purchase";

export function SalesReturnEntry({ onDone }: { onDone: () => void }) {
  return <ReturnEntry side="sales" onDone={onDone} />;
}

export function PurchaseReturnEntry({ onDone }: { onDone: () => void }) {
  return <ReturnEntry side="purchase" onDone={onDone} />;
}

function ReturnEntry({ side, onDone }: { side: Side; onDone: () => void }) {
  const copy = sideCopy(side);
  const [options, setOptions] = React.useState<ReturnOptionLine[]>([]);
  const [sourceId, setSourceId] = React.useState("");
  const [returnDate, setReturnDate] = React.useState(today());
  const [reasonCode, setReasonCode] = React.useState("QUALITY_ISSUE");
  const [businessNote, setBusinessNote] = React.useState("");
  const [quantities, setQuantities] = React.useState<Record<string, string>>(
    {},
  );
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");

  React.useEffect(() => {
    let active = true;
    request<ReturnOptions>(`/api/v1/${copy.resource}/options`)
      .then((result) => {
        if (!active) return;
        setOptions(result.items);
        setSourceId(result.items[0]?.sourceId ?? "");
        if (!result.canCreate)
          setNotice(`当前角色没有${copy.documentLabel}权限。`);
      })
      .catch((error: Error) => active && setNotice(error.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [copy.documentLabel, copy.resource]);

  const sources = uniqueSources(options);
  const lines = options.filter((line) => line.sourceId === sourceId);
  const selected = lines
    .map((line) => ({ line, quantity: quantities[line.sourceLineId] ?? "" }))
    .filter(({ quantity }) => number(quantity) > 0);
  const selectedSource = sources.find((source) => source.sourceId === sourceId);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setNotice("");
    if (!sourceId || selected.length === 0) {
      setNotice(`请选择来源${copy.sourceLabel}并填写至少一行退货数量。`);
      return;
    }
    if (
      selected.some(
        ({ line, quantity }) =>
          !positiveQuantity(quantity) ||
          number(quantity) > number(line.returnableQuantity),
      )
    ) {
      setNotice("退货数量必须大于 0，且不能超过来源单据可退数量。");
      return;
    }
    setBusy(true);
    try {
      await request(`/api/v1/${copy.resource}`, {
        method: "POST",
        body: JSON.stringify({
          sourceId,
          returnDate,
          reasonCode,
          businessNote: businessNote.trim() || undefined,
          lines: selected.map(({ line, quantity }) => ({
            sourceLineId: line.sourceLineId,
            quantity,
          })),
        }),
      });
      onDone();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className={`return-entry ${side}`}>
      <header>
        <div>
          <span>{copy.eyebrow}</span>
          <h3>新增{copy.documentLabel}</h3>
          <p>{copy.description}</p>
        </div>
        <strong>草稿</strong>
      </header>
      {loading ? (
        <p className="return-loading">正在核对可退来源与剩余数量…</p>
      ) : (
        <form onSubmit={submit}>
          <div className="return-fields">
            <Field label={`来源${copy.sourceLabel}`}>
              <select
                value={sourceId}
                onChange={(event) => {
                  setSourceId(event.target.value);
                  setQuantities({});
                }}
                required
              >
                {sources.map((source) => (
                  <option key={source.sourceId} value={source.sourceId}>
                    {source.sourceNumber} · {source.partnerName} ·{" "}
                    {source.warehouseName}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="退货日期">
              <input
                type="date"
                value={returnDate}
                onChange={(event) => setReturnDate(event.target.value)}
                required
              />
            </Field>
            <Field label="退货原因">
              <select
                value={reasonCode}
                onChange={(event) => setReasonCode(event.target.value)}
              >
                <option value="QUALITY_ISSUE">质量问题</option>
                <option value="WRONG_ITEM">错发 / 错收</option>
                <option value="DAMAGED">运输破损</option>
                <option value="COMMERCIAL_AGREEMENT">商业协商</option>
                <option value="OTHER">其他</option>
              </select>
            </Field>
          </div>
          {selectedSource && (
            <div className="return-source-strip">
              <span>
                关联订单 <strong>{selectedSource.orderNumber}</strong>
              </span>
              <span>
                {copy.partnerLabel}{" "}
                <strong>
                  {selectedSource.partnerCode} · {selectedSource.partnerName}
                </strong>
              </span>
              <span>
                退货仓库{" "}
                <strong>
                  {selectedSource.warehouseCode} ·{" "}
                  {selectedSource.warehouseName}
                </strong>
              </span>
            </div>
          )}
          <div className="return-lines">
            <div className="return-line-head">
              <span>商品</span>
              <span>来源数量</span>
              <span>已退 / 草稿占用</span>
              <span>本次退货</span>
              <span />
            </div>
            {lines.map((line) => (
              <div className="return-line" key={line.sourceLineId}>
                <span>
                  <strong>{line.skuCode}</strong>
                  <small>{line.skuName}</small>
                </span>
                <strong>{formatQuantity(line.sourceQuantity)}</strong>
                <span>
                  {formatQuantity(line.returnedQuantity)} / 可退{" "}
                  {formatQuantity(line.returnableQuantity)}
                </span>
                <input
                  aria-label={`${line.skuCode} 退货数量`}
                  inputMode="decimal"
                  value={quantities[line.sourceLineId] ?? ""}
                  onChange={(event) =>
                    setQuantities((current) => ({
                      ...current,
                      [line.sourceLineId]: event.target.value,
                    }))
                  }
                  placeholder="0"
                />
                <button
                  type="button"
                  className="secondary"
                  onClick={() =>
                    setQuantities((current) => ({
                      ...current,
                      [line.sourceLineId]: trim(line.returnableQuantity),
                    }))
                  }
                >
                  全部
                </button>
              </div>
            ))}
            {lines.length === 0 && (
              <div className="return-empty">
                暂无可退来源。只有已确认且仍有剩余数量的单据可以退货。
              </div>
            )}
          </div>
          <label className="return-note">
            <span>业务备注</span>
            <textarea
              value={businessNote}
              maxLength={1000}
              onChange={(event) => setBusinessNote(event.target.value)}
              placeholder="说明退货背景、检验结果或协商依据（可选）"
            />
          </label>
          <div className="return-effect">
            <ReturnIcon />
            <div>
              <strong>确认后的经营影响</strong>
              <span>{copy.effect}</span>
            </div>
          </div>
          {notice && (
            <p className="return-notice" role="alert">
              {notice}
            </p>
          )}
          <footer>
            <span>
              本次选择 {selected.length} 行；保存草稿不会改变库存与往来余额。
            </span>
            <button type="submit" disabled={busy || lines.length === 0}>
              {busy ? "正在保存…" : `保存${copy.documentLabel}草稿`}
            </button>
          </footer>
        </form>
      )}
    </section>
  );
}

function Field({
  label,
  children,
}: React.PropsWithChildren<{ label: string }>) {
  const id = React.useId();
  const control = React.Children.only(children) as React.ReactElement<{
    id?: string;
  }>;
  return (
    <label className="return-field" htmlFor={id}>
      <span>{label}</span>
      {React.cloneElement(control, { id })}
    </label>
  );
}

function sideCopy(side: Side) {
  return side === "sales"
    ? ({
        resource: "sales-returns",
        documentLabel: "销售退货单",
        sourceLabel: "销售出库单",
        partnerLabel: "客户",
        eyebrow: "SALES RETURN / GOODS IN",
        description: "按原出库凭据退回库存，并冲减对应未结经营应收。",
        effect:
          "库存按原出库冻结成本入库；经营应收、订单收入与产品成本同步冲减。",
      } as const)
    : ({
        resource: "purchase-returns",
        documentLabel: "采购退货单",
        sourceLabel: "采购收货单",
        partnerLabel: "供应商",
        eyebrow: "PURCHASE RETURN / GOODS OUT",
        description: "按原收货凭据退回供应商，并冲减对应未结经营应付。",
        effect:
          "库存按当前移动平均成本出库；经营应付按原收货价税金额同步冲减。",
      } as const);
}

function uniqueSources(lines: ReturnOptionLine[]) {
  return [...new Map(lines.map((line) => [line.sourceId, line])).values()];
}
function positiveQuantity(value: string) {
  return /^\d+(\.\d{1,6})?$/.test(value.trim()) && number(value) > 0;
}
function number(value: string) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}
function trim(value: string) {
  return String(number(value));
}
function today() {
  const date = new Date();
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
function ReturnIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
    >
      <path d="M8 7H4v4" />
      <path d="M4 11c1.6-4.6 6.6-7 11-5.2A8 8 0 1 1 6.2 18" />
      <path d="m4 11 4-4" />
    </svg>
  );
}
