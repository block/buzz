import React from "react";
import {
  type MasterDataList,
  type MasterDataRecord,
  type Payable,
  type Receipt,
  type Receivable,
  type SupplierPayment,
  request,
} from "./api";
import { formatAmount } from "./formatters";
import "./settlement-forms.css";

type SettlementSide = "customer" | "supplier";
type SettlementDocument = Receipt | SupplierPayment;
type OpenDocument = Receivable | Payable;
type CommandResult = { number: string; status: string; version: number };

export function CustomerReceiptEntry({ onDone }: { onDone: () => void }) {
  return <SettlementEntry side="customer" onDone={onDone} />;
}

export function SupplierPaymentEntry({ onDone }: { onDone: () => void }) {
  return <SettlementEntry side="supplier" onDone={onDone} />;
}

export function CustomerReceiptSettlement({
  receipt,
  receivables,
  onDone,
}: {
  receipt: Receipt;
  receivables: Receivable[];
  onDone: () => void;
}) {
  return (
    <SettlementAllocation
      side="customer"
      document={receipt}
      openDocuments={receivables}
      onDone={onDone}
    />
  );
}

export function SupplierPaymentSettlement({
  payment,
  payables,
  onDone,
}: {
  payment: SupplierPayment;
  payables: Payable[];
  onDone: () => void;
}) {
  return (
    <SettlementAllocation
      side="supplier"
      document={payment}
      openDocuments={payables}
      onDone={onDone}
    />
  );
}

function SettlementEntry({
  side,
  onDone,
}: {
  side: SettlementSide;
  onDone: () => void;
}) {
  const partnerResource = side === "customer" ? "customer" : "supplier";
  const [legalEntities, setLegalEntities] = React.useState<MasterDataRecord[]>(
    [],
  );
  const [partners, setPartners] = React.useState<MasterDataRecord[]>([]);
  const [legalEntityId, setLegalEntityId] = React.useState("");
  const [partnerId, setPartnerId] = React.useState("");
  const [businessDate, setBusinessDate] = React.useState(today());
  const [amount, setAmount] = React.useState("");
  const [paymentMethod, setPaymentMethod] = React.useState("bank_transfer");
  const [externalReference, setExternalReference] = React.useState("");
  const [businessNote, setBusinessNote] = React.useState("");
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");

  React.useEffect(() => {
    let active = true;
    Promise.all([loadMaster("legal_entity"), loadMaster(partnerResource)])
      .then(([entities, partnerRows]) => {
        if (!active) return;
        setLegalEntities(entities);
        setPartners(partnerRows);
        setLegalEntityId(entities[0]?.id ?? "");
        setPartnerId(scoped(partnerRows, entities[0]?.id ?? "")[0]?.id ?? "");
      })
      .catch((error: Error) => active && setNotice(error.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [partnerResource]);

  const availablePartners = scoped(partners, legalEntityId);
  const copy = sideCopy(side);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setNotice("");
    if (!legalEntityId || !partnerId) {
      setNotice(`请选择法律主体和${copy.partnerLabel}。`);
      return;
    }
    if (!positiveMoney(amount)) {
      setNotice(`${copy.amountLabel}必须大于 0，且最多保留 6 位小数。`);
      return;
    }
    setBusy(true);
    try {
      const result = await request<CommandResult>(copy.createPath, {
        method: "POST",
        body: JSON.stringify({
          legalEntityId,
          [copy.partnerKey]: partnerId,
          currency: "CNY",
          [copy.dateKey]: businessDate,
          amount,
          paymentMethod,
          externalReference: externalReference.trim() || undefined,
          businessNote: businessNote.trim() || undefined,
        }),
      });
      setNotice(`${copy.documentLabel} ${result.number} 已保存为草稿。`);
      onDone();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className={`settlement-form ${side}`}>
      <header>
        <div>
          <span>{copy.entryEyebrow}</span>
          <h3>登记{copy.documentLabel}</h3>
          <p>先保存草稿；确认后资金才可用于经营性核销。</p>
        </div>
        <strong>草稿</strong>
      </header>
      {loading ? (
        <p className="settlement-loading">正在加载可用主体与往来单位…</p>
      ) : (
        <form onSubmit={submit}>
          <div className="settlement-grid">
            <Field label="法律主体">
              <select
                value={legalEntityId}
                onChange={(event) => {
                  const value = event.target.value;
                  setLegalEntityId(value);
                  setPartnerId(scoped(partners, value)[0]?.id ?? "");
                }}
                required
              >
                {legalEntities.map(option)}
              </select>
            </Field>
            <Field label={copy.partnerLabel}>
              <select
                value={partnerId}
                onChange={(event) => setPartnerId(event.target.value)}
                required
              >
                {availablePartners.map(option)}
              </select>
            </Field>
            <Field label={copy.dateLabel}>
              <input
                type="date"
                value={businessDate}
                onChange={(event) => setBusinessDate(event.target.value)}
                required
              />
            </Field>
            <Field label={copy.amountLabel}>
              <div className="money-input">
                <span>CNY</span>
                <input
                  aria-label={copy.amountLabel}
                  inputMode="decimal"
                  value={amount}
                  onChange={(event) => setAmount(event.target.value)}
                  placeholder="0.00"
                  required
                />
              </div>
            </Field>
            <Field label="结算方式">
              <select
                value={paymentMethod}
                onChange={(event) => setPaymentMethod(event.target.value)}
              >
                <option value="bank_transfer">银行转账</option>
                <option value="cash">现金</option>
                <option value="commercial_draft">商业票据</option>
                <option value="other">其他</option>
              </select>
            </Field>
            <Field label="外部参考号">
              <input
                value={externalReference}
                maxLength={120}
                onChange={(event) => setExternalReference(event.target.value)}
                placeholder="银行流水号或回单号（可选）"
              />
            </Field>
          </div>
          <label className="settlement-note">
            <span>业务备注</span>
            <textarea
              value={businessNote}
              maxLength={500}
              onChange={(event) => setBusinessNote(event.target.value)}
              placeholder="仅记录经营说明，不生成会计凭证"
            />
          </label>
          <BoundaryNote />
          {notice && (
            <p className="settlement-notice" role="alert">
              {notice}
            </p>
          )}
          <footer>
            <span>保存后状态为草稿，可在台账中继续确认与核销。</span>
            <button type="submit" disabled={busy}>
              {busy ? "正在保存…" : `保存${copy.documentLabel}草稿`}
            </button>
          </footer>
        </form>
      )}
    </section>
  );
}

function SettlementAllocation({
  side,
  document,
  openDocuments,
  onDone,
}: {
  side: SettlementSide;
  document: SettlementDocument;
  openDocuments: OpenDocument[];
  onDone: () => void;
}) {
  const copy = sideCopy(side);
  const [current, setCurrent] = React.useState(document);
  const [amounts, setAmounts] = React.useState<Record<string, string>>({});
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  const eligible = openDocuments.filter(
    (row) =>
      row.legalEntityId === current.legalEntityId &&
      partnerId(row, side) === partnerId(current, side) &&
      row.currency === current.currency &&
      Number(row.openAmount) > 0 &&
      row.status !== "reversed",
  );
  const requested = Object.values(amounts).reduce(
    (sum, value) => sum + numeric(value),
    0,
  );
  const unapplied = numeric(current.unappliedAmount);
  const complete =
    current.status === "fully_allocated" || current.status === "reversed";

  async function confirm() {
    setBusy(true);
    setNotice("");
    try {
      const result = await request<CommandResult>(
        `${copy.createPath}/${current.id}/confirm`,
        {
          method: "POST",
          body: JSON.stringify({ expectedVersion: current.version }),
        },
      );
      setCurrent({
        ...current,
        status: result.status,
        version: result.version,
        unappliedAmount: current.amount,
      });
      setNotice(`${copy.documentLabel}已确认，现在可以选择待结业务单据。`);
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function allocate(event: React.FormEvent) {
    event.preventDefault();
    setNotice("");
    const selected = eligible
      .map((row) => ({ row, amount: amounts[row.id] ?? "" }))
      .filter(({ amount }) => numeric(amount) > 0);
    if (selected.length === 0) {
      setNotice(`请至少填写一笔${copy.openLabel}核销金额。`);
      return;
    }
    if (
      selected.some(
        ({ row, amount }) =>
          !positiveMoney(amount) || numeric(amount) > numeric(row.openAmount),
      )
    ) {
      setNotice(
        `核销金额必须大于 0，且不能超过对应${copy.openLabel}未结余额。`,
      );
      return;
    }
    if (requested > unapplied + 0.0000001) {
      setNotice("本次核销合计不能超过当前未核销资金。 ");
      return;
    }
    setBusy(true);
    try {
      await request(`${copy.createPath}/${current.id}/allocations`, {
        method: "POST",
        body: JSON.stringify({
          [copy.versionKey]: current.version,
          allocations: selected.map(({ row, amount }) => ({
            [copy.allocationKey]: row.id,
            amount,
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
    <section className={`settlement-form allocation ${side}`}>
      <header>
        <div>
          <span>{copy.settleEyebrow}</span>
          <h3>{documentNumber(current)}</h3>
          <p>资金确认与业务单据核销分步留痕，提交时执行版本校验。</p>
        </div>
        <strong>{statusLabel(current.status)}</strong>
      </header>
      <div className="settlement-balance">
        <Metric
          label={copy.amountLabel}
          value={current.amount}
          currency={current.currency}
        />
        <Metric
          label="累计已核销"
          value={current.allocatedAmount}
          currency={current.currency}
        />
        <Metric
          label="当前未核销"
          value={current.unappliedAmount}
          currency={current.currency}
          emphasis
        />
        <div
          className="balance-track"
          role="progressbar"
          aria-label="已核销比例"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={allocationPercent(current)}
        >
          <i style={{ width: `${allocationPercent(current)}%` }} />
        </div>
      </div>
      {current.status === "draft" ? (
        <div className="settlement-confirm-step">
          <div>
            <strong>01 · 确认资金事实</strong>
            <span>确认后金额、主体、往来方和币种不可在此修改。</span>
          </div>
          {notice && (
            <p className="settlement-notice" role="alert">
              {notice}
            </p>
          )}
          <button type="button" onClick={confirm} disabled={busy}>
            {busy ? "正在确认…" : `确认${copy.documentLabel}`}
          </button>
        </div>
      ) : complete ? (
        <div className="settlement-complete">
          <CheckIcon />
          <div>
            <strong>
              {current.status === "reversed"
                ? `${copy.documentLabel}已冲销`
                : "资金已全部核销"}
            </strong>
            <span>
              {current.status === "reversed"
                ? "该业务事实不再可用于核销。"
                : "当前无未分配余额，无需继续操作。"}
            </span>
          </div>
        </div>
      ) : (
        <form onSubmit={allocate}>
          <div className="allocation-heading">
            <div>
              <span>02 · OPERATING ALLOCATION</span>
              <h4>选择待结{copy.openLabel}</h4>
            </div>
            <strong>本次 {formatAmount(requested)}</strong>
          </div>
          <div className="allocation-list">
            {eligible.map((row) => {
              const max = Math.min(numeric(row.openAmount), unapplied);
              return (
                <label className="allocation-row" key={row.id}>
                  <span>
                    <strong>{openNumber(row)}</strong>
                    <small>
                      到期 {row.dueDate} · {statusLabel(row.status)}
                    </small>
                  </span>
                  <span>
                    <small>未结余额</small>
                    <strong>
                      {row.currency} {formatAmount(row.openAmount)}
                    </strong>
                  </span>
                  <input
                    aria-label={`${openNumber(row)} 核销金额`}
                    inputMode="decimal"
                    value={amounts[row.id] ?? ""}
                    onChange={(event) =>
                      setAmounts((currentAmounts) => ({
                        ...currentAmounts,
                        [row.id]: event.target.value,
                      }))
                    }
                    placeholder="0.00"
                  />
                  <button
                    type="button"
                    className="secondary"
                    onClick={() =>
                      setAmounts((currentAmounts) => ({
                        ...currentAmounts,
                        [row.id]: trimMoney(
                          Math.min(
                            max,
                            Math.max(
                              0,
                              unapplied -
                                requested +
                                numeric(currentAmounts[row.id] ?? "0"),
                            ),
                          ),
                        ),
                      }))
                    }
                  >
                    填满
                  </button>
                </label>
              );
            })}
            {eligible.length === 0 && (
              <div className="allocation-empty">
                当前没有与该资金同主体、同{copy.partnerLabel}、同币种的待结
                {copy.openLabel}。
              </div>
            )}
          </div>
          <div
            className={`allocation-check ${requested > unapplied ? "over" : ""}`}
          >
            <span>
              本次核销 {current.currency} {formatAmount(requested)}
            </span>
            <strong>
              提交后剩余 {current.currency}{" "}
              {formatAmount(Math.max(0, unapplied - requested))}
            </strong>
          </div>
          <BoundaryNote />
          {notice && (
            <p className="settlement-notice" role="alert">
              {notice}
            </p>
          )}
          <footer>
            <span>仅提交金额大于 0 的行；不会自动跨往来方分配。</span>
            <button
              type="submit"
              disabled={
                busy ||
                eligible.length === 0 ||
                requested <= 0 ||
                requested > unapplied
              }
            >
              {busy ? "正在核销…" : `确认核销 ${formatAmount(requested)}`}
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
    <label className="settlement-field" htmlFor={id}>
      <span>{label}</span>
      {React.cloneElement(control, { id })}
    </label>
  );
}

function Metric({
  label,
  value,
  currency,
  emphasis = false,
}: {
  label: string;
  value: string;
  currency: string;
  emphasis?: boolean;
}) {
  return (
    <div className={emphasis ? "emphasis" : ""}>
      <span>{label}</span>
      <strong>
        <small>{currency}</small> {formatAmount(value)}
      </strong>
    </div>
  );
}

function BoundaryNote() {
  return (
    <div className="settlement-boundary">
      <ShieldIcon />
      <span>
        <strong>经营边界</strong>{" "}
        这里记录收付款及其业务核销，不执行银行对账，也不生成会计总账凭证。
      </span>
    </div>
  );
}

function sideCopy(side: SettlementSide) {
  return side === "customer"
    ? ({
        partnerLabel: "客户",
        partnerKey: "customerId",
        dateKey: "receiptDate",
        dateLabel: "收款日期",
        amountLabel: "收款金额",
        documentLabel: "客户收款",
        openLabel: "应收",
        createPath: "/api/v1/customer-receipts",
        versionKey: "expectedReceiptVersion",
        allocationKey: "receivableId",
        entryEyebrow: "CUSTOMER RECEIPT / CASH IN",
        settleEyebrow: "RECEIPT SETTLEMENT / CONTROLLED",
      } as const)
    : ({
        partnerLabel: "供应商",
        partnerKey: "supplierId",
        dateKey: "paymentDate",
        dateLabel: "付款日期",
        amountLabel: "付款金额",
        documentLabel: "供应商付款",
        openLabel: "应付",
        createPath: "/api/v1/supplier-payments",
        versionKey: "expectedPaymentVersion",
        allocationKey: "payableId",
        entryEyebrow: "SUPPLIER PAYMENT / CASH OUT",
        settleEyebrow: "PAYMENT SETTLEMENT / CONTROLLED",
      } as const);
}

function partnerId(
  row: SettlementDocument | OpenDocument,
  side: SettlementSide,
) {
  return side === "customer"
    ? (row as Receipt | Receivable).customerId
    : (row as SupplierPayment | Payable).supplierId;
}
function documentNumber(row: SettlementDocument) {
  return "receiptNumber" in row ? row.receiptNumber : row.supplierPaymentNumber;
}
function openNumber(row: OpenDocument) {
  return "receivableNumber" in row ? row.receivableNumber : row.payableNumber;
}
function allocationPercent(row: SettlementDocument) {
  const total = numeric(row.amount);
  return total <= 0
    ? 0
    : Math.min(100, Math.round((numeric(row.allocatedAmount) / total) * 100));
}
function numeric(value: string) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}
function positiveMoney(value: string) {
  return /^\d+(\.\d{1,6})?$/.test(value.trim()) && numeric(value) > 0;
}
function trimMoney(value: number) {
  return value.toFixed(6).replace(/\.?0+$/, "");
}
function today() {
  const date = new Date();
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
function scoped(rows: MasterDataRecord[], legalEntityId: string) {
  return rows.filter(
    (row) => !row.legalEntityId || row.legalEntityId === legalEntityId,
  );
}
function option(row: MasterDataRecord) {
  return (
    <option key={row.id} value={row.id}>
      {row.code} · {row.name}
    </option>
  );
}
async function loadMaster(resource: string) {
  const result = await request<MasterDataList>(
    `/api/v1/master-data/${resource}?limit=200`,
  );
  return result.items.filter((row) => row.status === "active");
}
function statusLabel(value: string) {
  return (
    (
      {
        draft: "草稿",
        confirmed: "已确认",
        open: "未结",
        partially_settled: "部分结清",
        settled: "已结清",
        partially_allocated: "部分核销",
        fully_allocated: "已核销",
        reversed: "已冲销",
      } as Record<string, string>
    )[value] ?? value
  );
}

function ShieldIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
    >
      <path d="M12 3 5 6v5c0 4.6 2.8 8.1 7 10 4.2-1.9 7-5.4 7-10V6z" />
      <path d="m9 12 2 2 4-4" />
    </svg>
  );
}
function CheckIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
    >
      <circle cx="12" cy="12" r="9" />
      <path d="m8 12 2.5 2.5L16 9" />
    </svg>
  );
}
