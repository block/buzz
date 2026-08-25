import React from "react";
import { createPortal } from "react-dom";
import {
  type CoreMasterCommandResult,
  type CoreMasterDisableImpact,
  type CoreMasterList,
  type CoreMasterRecord,
  type CoreMasterType,
  request,
} from "./api";
import { formatMoney } from "./formatters";
import "./core-master-data.css";

const TYPES: Array<{
  id: CoreMasterType;
  label: string;
  short: string;
  description: string;
}> = [
  {
    id: "legal_entity",
    label: "法定主体",
    short: "LE",
    description: "签约与责任边界",
  },
  {
    id: "business_unit",
    label: "经营主体",
    short: "BU",
    description: "经营归属与核算口径",
  },
  {
    id: "customer",
    label: "客户",
    short: "CUST",
    description: "销售关系与信用条件",
  },
  {
    id: "supplier",
    label: "供应商",
    short: "SUP",
    description: "采购关系与付款条件",
  },
  {
    id: "warehouse",
    label: "仓库",
    short: "WH",
    description: "库存保管与履约节点",
  },
];

type FormState = {
  code: string;
  name: string;
  legalEntityId: string;
  businessUnitId: string;
  countryCode: string;
  functionalCurrency: string;
  registrationNumber: string;
  address: string;
  creditCurrency: string;
  creditLimitYuan: string;
  paymentTermsDays: string;
};

type ModalState =
  | { kind: "form"; type: CoreMasterType; record?: CoreMasterRecord }
  | { kind: "status"; record: CoreMasterRecord };

const EMPTY_FORM: FormState = {
  code: "",
  name: "",
  legalEntityId: "",
  businessUnitId: "",
  countryCode: "CN",
  functionalCurrency: "CNY",
  registrationNumber: "",
  address: "",
  creditCurrency: "CNY",
  creditLimitYuan: "0",
  paymentTermsDays: "30",
};

export function CoreMasterDataCenter() {
  const [activeType, setActiveType] =
    React.useState<CoreMasterType>("legal_entity");
  const [data, setData] = React.useState<CoreMasterList | null>(null);
  const [query, setQuery] = React.useState("");
  const [status, setStatus] = React.useState("all");
  const [modal, setModal] = React.useState<ModalState | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setData(
        await request<CoreMasterList>("/api/v1/core-master-data?limit=1000"),
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "核心数据加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  const current = React.useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return (data?.items ?? []).filter(
      (item) =>
        item.resourceType === activeType &&
        (status === "all" || item.status === status) &&
        (!needle ||
          `${item.code} ${item.name} ${item.legalEntityName ?? ""} ${item.businessUnitName ?? ""}`
            .toLocaleLowerCase()
            .includes(needle)),
    );
  }, [activeType, data, query, status]);

  const counts = React.useMemo(
    () =>
      Object.fromEntries(
        TYPES.map(({ id }) => [
          id,
          (data?.items ?? []).filter((item) => item.resourceType === id).length,
        ]),
      ) as Record<CoreMasterType, number>,
    [data],
  );
  const selected = TYPES.find((item) => item.id === activeType) ?? TYPES[0];

  return (
    <section className="page core-master-page">
      <div className="page-head core-master-head">
        <div>
          <p>CORE DATA / AUTHORITATIVE REGISTER</p>
          <h1>核心数据中心</h1>
          <span>
            以法定主体 → 经营主体 →
            业务对象为统一关系主线，维护业务闭环依赖的权威基础数据。
          </span>
        </div>
        {data?.canManage && (
          <button
            type="button"
            onClick={() => setModal({ kind: "form", type: activeType })}
          >
            ＋ 新增{selected.label}
          </button>
        )}
      </div>

      <div
        className="master-spine"
        role="img"
        aria-label="主数据关系结构：法定主体到经营主体，再到客户、供应商与仓库"
      >
        <div>
          <b>01</b>
          <span>法定主体</span>
          <small>LEGAL OWNER</small>
        </div>
        <i>→</i>
        <div>
          <b>02</b>
          <span>经营主体</span>
          <small>OPERATING UNIT</small>
        </div>
        <i>→</i>
        <div>
          <b>03</b>
          <span>客户 · 供应商 · 仓库</span>
          <small>OPERATING OBJECTS</small>
        </div>
      </div>

      <div className="master-tabs" role="tablist" aria-label="核心数据类别">
        {TYPES.map((item) => (
          <button
            type="button"
            role="tab"
            aria-selected={activeType === item.id}
            className={activeType === item.id ? "active" : ""}
            key={item.id}
            onClick={() => setActiveType(item.id)}
          >
            <b>{item.short}</b>
            <span>{item.label}</span>
            <em>{counts[item.id]}</em>
            <small>{item.description}</small>
          </button>
        ))}
      </div>

      <div className="master-toolbar">
        <label>
          <span>检索</span>
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="编码、名称或上级主体"
          />
        </label>
        <label>
          <span>状态</span>
          <select
            value={status}
            onChange={(event) => setStatus(event.target.value)}
          >
            <option value="all">全部状态</option>
            <option value="active">启用</option>
            <option value="disabled">停用</option>
          </select>
        </label>
        <button
          type="button"
          className="master-secondary"
          onClick={() => void load()}
        >
          刷新
        </button>
      </div>

      {error && <div className="master-message error">{error}</div>}
      {loading ? (
        <div className="master-message">正在读取权威主数据…</div>
      ) : current.length === 0 ? (
        <div className="master-empty">
          <b>{selected.short}</b>
          <h2>尚无符合条件的{selected.label}</h2>
          <p>清除筛选条件，或通过右上角按钮新增第一条权威记录。</p>
        </div>
      ) : (
        <div className="master-register">
          <div className="master-register-head">
            <span>编码 / 名称</span>
            <span>权威关系</span>
            <span>业务属性</span>
            <span>状态 / 版本</span>
            <span>操作</span>
          </div>
          {current.map((item) => (
            <article
              key={item.id}
              className={item.status === "disabled" ? "disabled" : ""}
            >
              <div className="master-identity">
                <code>{item.code}</code>
                <strong>{item.name}</strong>
                <small>更新 {formatDate(item.updatedAt)}</small>
              </div>
              <Hierarchy item={item} />
              <div className="master-attribute">
                <strong>{attribute(item)}</strong>
                <small>{attributeNote(item)}</small>
              </div>
              <div>
                <span className={`master-status ${item.status}`}>
                  {item.status === "active" ? "启用" : "停用"}
                </span>
                <small className="master-version">VERSION {item.version}</small>
              </div>
              <div className="master-actions">
                {data?.canManage ? (
                  <>
                    <button
                      type="button"
                      onClick={() =>
                        setModal({
                          kind: "form",
                          type: item.resourceType,
                          record: item,
                        })
                      }
                    >
                      编辑
                    </button>
                    <button
                      type="button"
                      className={
                        item.status === "active" ? "danger" : "activate"
                      }
                      onClick={() => setModal({ kind: "status", record: item })}
                    >
                      {item.status === "active" ? "停用" : "启用"}
                    </button>
                  </>
                ) : (
                  <small>只读权限</small>
                )}
              </div>
            </article>
          ))}
        </div>
      )}

      <footer className="master-footnote">
        <span>DATA AS OF {data ? formatDate(data.dataAsOf) : "—"}</span>
        <p>
          编码与归属关系创建后保持不变；停用前实时检查库存、订单、往来对象等业务影响。
        </p>
      </footer>

      {modal?.kind === "form" && (
        <MasterFormModal
          state={modal}
          items={data?.items ?? []}
          onClose={() => setModal(null)}
          onSaved={async () => {
            setModal(null);
            await load();
          }}
        />
      )}
      {modal?.kind === "status" && (
        <StatusModal
          record={modal.record}
          onClose={() => setModal(null)}
          onSaved={async () => {
            setModal(null);
            await load();
          }}
        />
      )}
    </section>
  );
}

function Hierarchy({ item }: { item: CoreMasterRecord }) {
  const steps =
    item.resourceType === "legal_entity"
      ? [{ id: "legal", name: item.name }]
      : item.resourceType === "business_unit"
        ? [
            { id: "legal", name: item.legalEntityName },
            { id: "unit", name: item.name },
          ]
        : [
            { id: "legal", name: item.legalEntityName },
            { id: "unit", name: item.businessUnitName },
            { id: "object", name: item.name },
          ];
  const visible = steps.filter((step) => step.name);
  return (
    <div className="master-hierarchy">
      {visible.map((step, index) => (
        <React.Fragment key={step.id}>
          <span>{step.name}</span>
          {index < visible.length - 1 && <i>›</i>}
        </React.Fragment>
      ))}
    </div>
  );
}

function MasterFormModal({
  state,
  items,
  onClose,
  onSaved,
}: {
  state: Extract<ModalState, { kind: "form" }>;
  items: CoreMasterRecord[];
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const { record, type } = state;
  const [form, setForm] = React.useState<FormState>(() =>
    record ? fromRecord(record) : EMPTY_FORM,
  );
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const entities = items.filter(
    (item) => item.resourceType === "legal_entity" && item.status === "active",
  );
  const units = items.filter(
    (item) =>
      item.resourceType === "business_unit" &&
      item.status === "active" &&
      (!form.legalEntityId || item.legalEntityId === form.legalEntityId),
  );
  const title = `${record ? "编辑" : "新增"}${labelFor(type)}`;
  const set = (field: keyof FormState, value: string) =>
    setForm((current) => ({ ...current, [field]: value }));

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    const payload = {
      resourceType: type,
      code: form.code.trim().toUpperCase(),
      name: form.name.trim(),
      legalEntityId: form.legalEntityId || null,
      businessUnitId: form.businessUnitId || null,
      countryCode: form.countryCode.trim().toUpperCase() || null,
      functionalCurrency: form.functionalCurrency.trim().toUpperCase() || null,
      registrationNumber: form.registrationNumber.trim() || null,
      address: form.address.trim() || null,
      creditCurrency: form.creditCurrency.trim().toUpperCase() || null,
      creditLimitMinor: Math.round(Number(form.creditLimitYuan || 0) * 100),
      paymentTermsDays: Number(form.paymentTermsDays || 30),
      expectedVersion: record?.version ?? null,
    };
    try {
      const path = record
        ? `/api/v1/core-master-data/${type}/${record.id}`
        : "/api/v1/core-master-data";
      await request<CoreMasterCommandResult>(path, {
        method: record ? "PUT" : "POST",
        body: JSON.stringify(payload),
      });
      await onSaved();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "保存失败");
    } finally {
      setSaving(false);
    }
  }

  return (
    <MasterModal
      title={title}
      eyebrow="CONTROLLED MASTER DATA"
      onClose={onClose}
    >
      <form className="master-form" onSubmit={submit}>
        <div className="master-form-note">
          <b>{record ? "受控修订" : "建立权威记录"}</b>
          <span>
            {record
              ? "编码与归属关系不可更改；保存时校验当前版本。"
              : "编码保存后不可更改，请确认所属关系准确。"}
          </span>
        </div>
        <div className="master-form-grid">
          <Field label="编码 *">
            <input
              required
              disabled={Boolean(record)}
              value={form.code}
              onChange={(e) => set("code", e.target.value.toUpperCase())}
              pattern="[A-Z0-9][A-Z0-9_-]*"
            />
          </Field>
          <Field label="名称 *">
            <input
              required
              value={form.name}
              onChange={(e) => set("name", e.target.value)}
            />
          </Field>
          {type !== "legal_entity" && (
            <Field label="法定主体 *">
              <select
                required
                disabled={Boolean(record)}
                value={form.legalEntityId}
                onChange={(e) => {
                  set("legalEntityId", e.target.value);
                  set("businessUnitId", "");
                }}
              >
                <option value="">请选择</option>
                {entities.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.code} · {item.name}
                  </option>
                ))}
              </select>
            </Field>
          )}
          {!(["legal_entity", "business_unit"] as CoreMasterType[]).includes(
            type,
          ) && (
            <Field label="经营主体 *">
              <select
                required
                disabled={Boolean(record)}
                value={form.businessUnitId}
                onChange={(e) => set("businessUnitId", e.target.value)}
              >
                <option value="">请选择</option>
                {units.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.code} · {item.name}
                  </option>
                ))}
              </select>
            </Field>
          )}
          {type === "legal_entity" && (
            <>
              <Field label="国家/地区">
                <input
                  maxLength={2}
                  value={form.countryCode}
                  onChange={(e) => set("countryCode", e.target.value)}
                />
              </Field>
              <Field label="功能币">
                <input
                  maxLength={3}
                  value={form.functionalCurrency}
                  onChange={(e) => set("functionalCurrency", e.target.value)}
                />
              </Field>
              <Field label="登记编号" wide>
                <input
                  value={form.registrationNumber}
                  onChange={(e) => set("registrationNumber", e.target.value)}
                />
              </Field>
            </>
          )}
          {type === "customer" && (
            <>
              <Field label="信用币种">
                <input
                  maxLength={3}
                  value={form.creditCurrency}
                  onChange={(e) => set("creditCurrency", e.target.value)}
                />
              </Field>
              <Field label="信用额度（元）">
                <input
                  type="number"
                  min="0"
                  step="0.01"
                  value={form.creditLimitYuan}
                  onChange={(e) => set("creditLimitYuan", e.target.value)}
                />
              </Field>
              <TermsField value={form.paymentTermsDays} set={set} />
            </>
          )}
          {type === "supplier" && (
            <TermsField value={form.paymentTermsDays} set={set} />
          )}
          {type === "warehouse" && (
            <Field label="地址" wide>
              <textarea
                rows={3}
                value={form.address}
                onChange={(e) => set("address", e.target.value)}
              />
            </Field>
          )}
        </div>
        {error && <p className="master-form-error">{error}</p>}
        <div className="master-form-actions">
          <button type="button" className="master-secondary" onClick={onClose}>
            取消
          </button>
          <button type="submit" disabled={saving}>
            {saving ? "保存中…" : record ? "保存修订" : "确认新增"}
          </button>
        </div>
      </form>
    </MasterModal>
  );
}

function TermsField({
  value,
  set,
}: {
  value: string;
  set: (field: keyof FormState, value: string) => void;
}) {
  return (
    <Field label="付款账期（天）">
      <input
        type="number"
        min="0"
        max="3650"
        value={value}
        onChange={(e) => set("paymentTermsDays", e.target.value)}
      />
    </Field>
  );
}

function Field({
  label,
  wide,
  children,
}: React.PropsWithChildren<{ label: string; wide?: boolean }>) {
  const id = React.useId();
  return (
    <label className={wide ? "wide" : ""} htmlFor={id}>
      <span>{label}</span>
      {React.cloneElement(children as React.ReactElement<{ id?: string }>, {
        id,
      })}
    </label>
  );
}

function StatusModal({
  record,
  onClose,
  onSaved,
}: {
  record: CoreMasterRecord;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const disabling = record.status === "active";
  const [impact, setImpact] = React.useState<CoreMasterDisableImpact | null>(
    null,
  );
  const [loading, setLoading] = React.useState(disabling);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!disabling) return;
    request<CoreMasterDisableImpact>(
      `/api/v1/core-master-data/${record.resourceType}/${record.id}/disable-impact`,
    )
      .then(setImpact)
      .catch((reason: Error) => setError(reason.message))
      .finally(() => setLoading(false));
  }, [disabling, record]);
  async function confirm() {
    setSaving(true);
    setError(null);
    try {
      await request<CoreMasterCommandResult>(
        `/api/v1/core-master-data/${record.resourceType}/${record.id}/status`,
        {
          method: "POST",
          body: JSON.stringify({
            status: disabling ? "disabled" : "active",
            expectedVersion: record.version,
          }),
        },
      );
      await onSaved();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "状态变更失败");
    } finally {
      setSaving(false);
    }
  }
  const allowed = !disabling || impact?.canDisable === true;
  return (
    <MasterModal
      title={`${disabling ? "停用" : "启用"}${labelFor(record.resourceType)}`}
      eyebrow="STATUS CONTROL / IMPACT CHECK"
      onClose={onClose}
    >
      <div className="impact-panel">
        <div className="impact-target">
          <code>{record.code}</code>
          <h3>{record.name}</h3>
          <p>
            {disabling
              ? "停用后不可用于新业务单据。系统已按实时业务事实执行影响检查。"
              : "启用后可重新用于新业务单据，历史记录不受影响。"}
          </p>
        </div>
        {loading && (
          <div className="master-message">正在检查订单、库存与往来影响…</div>
        )}
        {impact && (
          <div className="impact-list">
            {impact.impacts.map((item) => (
              <div
                key={item.code}
                className={
                  item.blocking && item.count > 0 ? "blocked" : "clear"
                }
              >
                <b>{item.count}</b>
                <span>
                  {item.label}
                  <small>{item.blocking ? "阻断项" : "提示项"}</small>
                </span>
              </div>
            ))}
          </div>
        )}
        {impact && (
          <div
            className={`impact-decision ${impact.canDisable ? "ready" : "blocked"}`}
          >
            <b>{impact.canDisable ? "可以停用" : "暂不可停用"}</b>
            <span>
              {impact.canDisable
                ? "未发现仍在运行的阻断业务。"
                : "请先处理标红的业务影响，再重新检查。"}
            </span>
          </div>
        )}
        {error && <p className="master-form-error">{error}</p>}
        <div className="master-form-actions">
          <button type="button" className="master-secondary" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className={disabling ? "master-danger" : ""}
            disabled={!allowed || saving}
            onClick={() => void confirm()}
          >
            {saving ? "处理中…" : `确认${disabling ? "停用" : "启用"}`}
          </button>
        </div>
      </div>
    </MasterModal>
  );
}

export function MasterModal({
  title,
  eyebrow,
  onClose,
  children,
}: React.PropsWithChildren<{
  title: string;
  eyebrow: string;
  onClose: () => void;
}>) {
  const panel = React.useRef<HTMLDivElement>(null);
  const trigger = React.useRef<HTMLElement | null>(null);
  React.useEffect(() => {
    trigger.current = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    panel.current
      ?.querySelector<HTMLElement>("input, select, textarea, button")
      ?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      if (event.key !== "Tab" || !panel.current) return;
      const focusable = [
        ...panel.current.querySelectorAll<HTMLElement>(
          "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled])",
        ),
      ];
      const first = focusable[0];
      const last = focusable.at(-1);
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };
    document.addEventListener("keydown", keydown);
    return () => {
      document.body.style.overflow = overflow;
      document.removeEventListener("keydown", keydown);
      trigger.current?.focus();
    };
  }, [onClose]);
  return createPortal(
    <div className="master-modal-layer">
      <button
        type="button"
        aria-label="关闭弹窗"
        className="master-modal-scrim"
        onClick={onClose}
      />
      <div
        className="master-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="master-modal-title"
        ref={panel}
      >
        <header>
          <div>
            <span>{eyebrow}</span>
            <h2 id="master-modal-title">{title}</h2>
          </div>
          <button type="button" aria-label="关闭弹窗" onClick={onClose}>
            ×
          </button>
        </header>
        <div className="master-modal-body">{children}</div>
      </div>
    </div>,
    document.body,
  );
}

function fromRecord(record: CoreMasterRecord): FormState {
  return {
    code: record.code,
    name: record.name,
    legalEntityId: record.legalEntityId ?? "",
    businessUnitId: record.businessUnitId ?? "",
    countryCode: record.countryCode ?? "CN",
    functionalCurrency: record.functionalCurrency ?? "CNY",
    registrationNumber: record.registrationNumber ?? "",
    address: record.address ?? "",
    creditCurrency: record.creditCurrency ?? "CNY",
    creditLimitYuan: String((record.creditLimitMinor ?? 0) / 100),
    paymentTermsDays: String(record.paymentTermsDays ?? 30),
  };
}
function labelFor(type: CoreMasterType) {
  return TYPES.find((item) => item.id === type)?.label ?? type;
}
function attribute(item: CoreMasterRecord) {
  if (item.resourceType === "legal_entity")
    return `${item.countryCode ?? "—"} · ${item.functionalCurrency ?? "—"}`;
  if (item.resourceType === "customer")
    return `信用 ${formatMoney("CNY", (item.creditLimitMinor ?? 0) / 100)}`;
  if (item.resourceType === "supplier")
    return `${item.paymentTermsDays ?? 0} 天账期`;
  if (item.resourceType === "warehouse") return item.address || "地址待维护";
  return "经营归属节点";
}
function attributeNote(item: CoreMasterRecord) {
  if (item.resourceType === "legal_entity")
    return item.registrationNumber || "登记编号待维护";
  if (item.resourceType === "customer")
    return `${item.paymentTermsDays ?? 0} 天账期 · ${item.creditCurrency ?? "CNY"}`;
  return item.resourceType === "business_unit"
    ? "承接客户、供应商与仓库"
    : "受主体关系约束";
}
function formatDate(value: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(new Date(value));
}
