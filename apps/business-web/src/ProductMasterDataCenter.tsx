import React from "react";
import {
  type ProductMasterCommandResult,
  type ProductMasterDisableImpact,
  type ProductMasterList,
  type ProductMasterRecord,
  type ProductMasterType,
  request,
} from "./api";
import { MasterModal } from "./CoreMasterDataCenter";
import "./product-master-data.css";

const TYPES: Array<{
  id: ProductMasterType;
  label: string;
  short: string;
  description: string;
}> = [
  {
    id: "product",
    label: "商品",
    short: "SPU",
    description: "分类、品牌与基础单位",
  },
  {
    id: "sku",
    label: "SKU / 条码",
    short: "SKU",
    description: "可交易库存对象",
  },
  {
    id: "product_category",
    label: "商品分类",
    short: "CAT",
    description: "层级经营口径",
  },
  {
    id: "brand",
    label: "品牌",
    short: "BRD",
    description: "品牌归属与分析口径",
  },
  {
    id: "unit_of_measure",
    label: "计量单位",
    short: "UOM",
    description: "数量精度规则",
  },
  {
    id: "uom_conversion",
    label: "单位换算",
    short: "CVT",
    description: "采购与销售换算",
  },
];

type FormState = {
  code: string;
  name: string;
  parentCategoryId: string;
  categoryId: string;
  brandId: string;
  baseUomId: string;
  productId: string;
  unitOfMeasureId: string;
  barcode: string;
  precisionScale: string;
  allowZeroCost: boolean;
  factorToBase: string;
  usageScope: "sales" | "purchase" | "both";
};

type ModalState =
  | { kind: "form"; type: ProductMasterType; record?: ProductMasterRecord }
  | { kind: "status"; record: ProductMasterRecord };

const EMPTY_FORM: FormState = {
  code: "",
  name: "",
  parentCategoryId: "",
  categoryId: "",
  brandId: "",
  baseUomId: "",
  productId: "",
  unitOfMeasureId: "",
  barcode: "",
  precisionScale: "2",
  allowZeroCost: false,
  factorToBase: "1",
  usageScope: "both",
};

export function ProductMasterDataCenter() {
  const [activeType, setActiveType] =
    React.useState<ProductMasterType>("product");
  const [data, setData] = React.useState<ProductMasterList | null>(null);
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
        await request<ProductMasterList>(
          "/api/v1/product-master-data?limit=2000",
        ),
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "商品主数据加载失败");
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
          `${item.code} ${item.name} ${item.productName ?? ""} ${item.categoryName ?? ""} ${item.brandName ?? ""} ${item.barcode ?? ""}`
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
      ) as Record<ProductMasterType, number>,
    [data],
  );
  const selected = TYPES.find((item) => item.id === activeType) ?? TYPES[0];

  return (
    <section className="page core-master-page product-master-page">
      <div className="page-head core-master-head">
        <div>
          <p>PRODUCT DATA / CONTROLLED CATALOG</p>
          <h1>商品主数据中心</h1>
          <span>
            以分类、品牌和计量规则定义商品，再以
            SKU、条码和单位换算承接真实销售、采购与库存。
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
        className="master-spine product-spine"
        role="img"
        aria-label="商品主数据关系结构：分类、品牌和单位定义商品，商品下建立 SKU、条码及单位换算"
      >
        <div>
          <b>01</b>
          <span>分类 · 品牌 · 单位</span>
          <small>CATALOG RULES</small>
        </div>
        <i>→</i>
        <div>
          <b>02</b>
          <span>商品 / SPU</span>
          <small>PRODUCT DEFINITION</small>
        </div>
        <i>→</i>
        <div>
          <b>03</b>
          <span>SKU · 条码 · 换算</span>
          <small>TRADE OBJECTS</small>
        </div>
      </div>

      <div
        className="master-tabs product-tabs"
        role="tablist"
        aria-label="商品数据类别"
      >
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
            placeholder="编码、名称、条码、分类或品牌"
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
        <div className="master-message">正在读取商品权威数据…</div>
      ) : current.length === 0 ? (
        <div className="master-empty">
          <b>{selected.short}</b>
          <h2>尚无符合条件的{selected.label}</h2>
          <p>清除筛选条件，或新增第一条权威记录。</p>
        </div>
      ) : (
        <ProductRegister
          items={current}
          canManage={Boolean(data?.canManage)}
          onEdit={(record) =>
            setModal({ kind: "form", type: record.resourceType, record })
          }
          onStatus={(record) => setModal({ kind: "status", record })}
        />
      )}

      <footer className="master-footnote">
        <span>DATA AS OF {data ? formatDate(data.dataAsOf) : "—"}</span>
        <p>
          编码、所属商品和基础关系创建后不可更改；停用前实时检查
          SKU、订单与库存影响。
        </p>
      </footer>
      {modal?.kind === "form" && (
        <ProductFormModal
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
        <ProductStatusModal
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

function ProductRegister({
  items,
  canManage,
  onEdit,
  onStatus,
}: {
  items: ProductMasterRecord[];
  canManage: boolean;
  onEdit: (record: ProductMasterRecord) => void;
  onStatus: (record: ProductMasterRecord) => void;
}) {
  return (
    <div className="master-register product-register">
      <div className="master-register-head">
        <span>编码 / 名称</span>
        <span>商品关系</span>
        <span>识别与计量</span>
        <span>状态 / 版本</span>
        <span>操作</span>
      </div>
      {items.map((item) => (
        <article
          key={item.id}
          className={item.status === "disabled" ? "disabled" : ""}
        >
          <div className="master-identity">
            <code>{item.code}</code>
            <strong>{item.name}</strong>
            <small>更新 {formatDate(item.updatedAt)}</small>
          </div>
          <ProductHierarchy item={item} />
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
            {canManage ? (
              <>
                <button type="button" onClick={() => onEdit(item)}>
                  编辑
                </button>
                <button
                  type="button"
                  className={item.status === "active" ? "danger" : "activate"}
                  onClick={() => onStatus(item)}
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
  );
}

function ProductHierarchy({ item }: { item: ProductMasterRecord }) {
  let steps: Array<{ id: string; name: string | null }>;
  if (item.resourceType === "product_category")
    steps = [
      { id: "parent", name: item.parentCategoryName },
      { id: "category", name: item.name },
    ];
  else if (item.resourceType === "product")
    steps = [
      { id: "category", name: item.categoryName },
      { id: "product", name: item.name },
    ];
  else if (item.resourceType === "sku")
    steps = [
      { id: "product", name: item.productName },
      { id: "sku", name: item.name },
    ];
  else if (item.resourceType === "uom_conversion")
    steps = [
      { id: "product", name: item.productName },
      { id: "unit", name: item.unitOfMeasureName },
    ];
  else steps = [{ id: "self", name: item.name }];
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

function ProductFormModal({
  state,
  items,
  onClose,
  onSaved,
}: {
  state: Extract<ModalState, { kind: "form" }>;
  items: ProductMasterRecord[];
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const { type, record } = state;
  const [form, setForm] = React.useState<FormState>(() =>
    record ? fromRecord(record) : EMPTY_FORM,
  );
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const set = <K extends keyof FormState>(field: K, value: FormState[K]) =>
    setForm((current) => ({ ...current, [field]: value }));
  const categories = items.filter(
    (item) =>
      item.resourceType === "product_category" && item.status === "active",
  );
  const brands = items.filter(
    (item) => item.resourceType === "brand" && item.status === "active",
  );
  const units = items.filter(
    (item) =>
      item.resourceType === "unit_of_measure" && item.status === "active",
  );
  const products = items.filter(
    (item) => item.resourceType === "product" && item.status === "active",
  );

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    const conversionProduct = products.find(
      (item) => item.id === form.productId,
    );
    const conversionUnit = units.find(
      (item) => item.id === form.unitOfMeasureId,
    );
    const payload = {
      resourceType: type,
      code:
        type === "uom_conversion"
          ? `${conversionProduct?.code ?? "PRODUCT"}_${conversionUnit?.code ?? "UOM"}`
          : form.code.trim().toUpperCase(),
      name:
        type === "uom_conversion"
          ? `${conversionProduct?.name ?? "商品"} / ${conversionUnit?.name ?? "单位"}`
          : form.name.trim(),
      parentCategoryId: form.parentCategoryId || null,
      categoryId: form.categoryId || null,
      brandId: form.brandId || null,
      baseUomId: form.baseUomId || null,
      productId: form.productId || null,
      unitOfMeasureId: form.unitOfMeasureId || null,
      barcode: form.barcode.trim() || null,
      precisionScale: Number(form.precisionScale),
      allowZeroCost: form.allowZeroCost,
      factorToBase: form.factorToBase,
      usageScope: form.usageScope,
      expectedVersion: record?.version ?? null,
    };
    try {
      const path = record
        ? `/api/v1/product-master-data/${type}/${record.id}`
        : "/api/v1/product-master-data";
      await request<ProductMasterCommandResult>(path, {
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

  const immutable = Boolean(record);
  return (
    <MasterModal
      title={`${record ? "编辑" : "新增"}${labelFor(type)}`}
      eyebrow="CONTROLLED PRODUCT DATA"
      onClose={onClose}
    >
      <form className="master-form product-master-form" onSubmit={submit}>
        <div className="master-form-note">
          <b>{record ? "受控修订" : "建立商品权威记录"}</b>
          <span>
            {record
              ? "编码和所属关系不可更改；保存时校验当前版本。"
              : "编码与所属关系保存后不可更改，请确认定义准确。"}
          </span>
        </div>
        <div className="master-form-grid">
          {type !== "uom_conversion" && (
            <>
              <Field label="编码 *">
                <input
                  required
                  disabled={immutable}
                  value={form.code}
                  onChange={(event) =>
                    set("code", event.target.value.toUpperCase())
                  }
                  pattern="[A-Z0-9][A-Z0-9_-]*"
                />
              </Field>
              <Field label="名称 *">
                <input
                  required
                  value={form.name}
                  onChange={(event) => set("name", event.target.value)}
                />
              </Field>
            </>
          )}
          {type === "unit_of_measure" && (
            <Field label="数量精度">
              <select
                disabled={immutable}
                value={form.precisionScale}
                onChange={(event) => set("precisionScale", event.target.value)}
              >
                {[0, 1, 2, 3, 4, 5, 6].map((value) => (
                  <option key={value} value={value}>
                    {value} 位小数
                  </option>
                ))}
              </select>
            </Field>
          )}
          {type === "product_category" && (
            <Field label="上级分类">
              <select
                disabled={immutable}
                value={form.parentCategoryId}
                onChange={(event) =>
                  set("parentCategoryId", event.target.value)
                }
              >
                <option value="">顶级分类</option>
                {categories
                  .filter((item) => item.id !== record?.id)
                  .map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.code} · {item.name}
                    </option>
                  ))}
              </select>
            </Field>
          )}
          {type === "product" && (
            <>
              <Field label="商品分类 *">
                <select
                  required
                  disabled={immutable}
                  value={form.categoryId}
                  onChange={(event) => set("categoryId", event.target.value)}
                >
                  <option value="">请选择</option>
                  {categories.map(option)}
                </select>
              </Field>
              <Field label="品牌">
                <select
                  disabled={immutable}
                  value={form.brandId}
                  onChange={(event) => set("brandId", event.target.value)}
                >
                  <option value="">无品牌</option>
                  {brands.map(option)}
                </select>
              </Field>
              <Field label="基础单位 *">
                <select
                  required
                  disabled={immutable}
                  value={form.baseUomId}
                  onChange={(event) => set("baseUomId", event.target.value)}
                >
                  <option value="">请选择</option>
                  {units.map(option)}
                </select>
              </Field>
              <label className="product-check">
                <input
                  type="checkbox"
                  checked={form.allowZeroCost}
                  onChange={(event) =>
                    set("allowZeroCost", event.target.checked)
                  }
                />
                <span>允许零成本入库（例外策略）</span>
              </label>
            </>
          )}
          {type === "sku" && (
            <>
              <Field label="所属商品 *">
                <select
                  required
                  disabled={immutable}
                  value={form.productId}
                  onChange={(event) => set("productId", event.target.value)}
                >
                  <option value="">请选择</option>
                  {products.map(option)}
                </select>
              </Field>
              <Field label="条码">
                <input
                  value={form.barcode}
                  onChange={(event) => set("barcode", event.target.value)}
                  placeholder="EAN / UPC / 内部条码"
                />
              </Field>
            </>
          )}
          {type === "uom_conversion" && (
            <>
              <Field label="商品 *">
                <select
                  required
                  disabled={immutable}
                  value={form.productId}
                  onChange={(event) => {
                    set("productId", event.target.value);
                    set("unitOfMeasureId", "");
                  }}
                >
                  <option value="">请选择</option>
                  {products.map(option)}
                </select>
              </Field>
              <Field label="换算单位 *">
                <select
                  required
                  disabled={immutable}
                  value={form.unitOfMeasureId}
                  onChange={(event) =>
                    set("unitOfMeasureId", event.target.value)
                  }
                >
                  <option value="">请选择</option>
                  {units
                    .filter(
                      (item) =>
                        item.id !==
                        products.find(
                          (product) => product.id === form.productId,
                        )?.unitOfMeasureId,
                    )
                    .map(option)}
                </select>
              </Field>
              <Field label="折合基础单位数量 *">
                <input
                  required
                  type="number"
                  min="0.00000001"
                  step="0.00000001"
                  value={form.factorToBase}
                  onChange={(event) => set("factorToBase", event.target.value)}
                />
              </Field>
              <Field label="适用业务">
                <select
                  value={form.usageScope}
                  onChange={(event) =>
                    set(
                      "usageScope",
                      event.target.value as FormState["usageScope"],
                    )
                  }
                >
                  <option value="both">采购与销售</option>
                  <option value="purchase">仅采购</option>
                  <option value="sales">仅销售</option>
                </select>
              </Field>
              <div className="conversion-preview">
                <b>
                  1{" "}
                  {units.find((item) => item.id === form.unitOfMeasureId)
                    ?.name ?? "换算单位"}
                </b>
                <span>
                  = {form.factorToBase || "—"}{" "}
                  {products.find((item) => item.id === form.productId)
                    ?.unitOfMeasureName ?? "基础单位"}
                </span>
              </div>
            </>
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

function Field({
  label,
  children,
}: React.PropsWithChildren<{ label: string }>) {
  const id = React.useId();
  return (
    <label htmlFor={id}>
      <span>{label}</span>
      {React.cloneElement(children as React.ReactElement<{ id?: string }>, {
        id,
      })}
    </label>
  );
}
function option(item: ProductMasterRecord) {
  return (
    <option key={item.id} value={item.id}>
      {item.code} · {item.name}
    </option>
  );
}

function ProductStatusModal({
  record,
  onClose,
  onSaved,
}: {
  record: ProductMasterRecord;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const disabling = record.status === "active";
  const [impact, setImpact] = React.useState<ProductMasterDisableImpact | null>(
    null,
  );
  const [loading, setLoading] = React.useState(disabling);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!disabling) return;
    request<ProductMasterDisableImpact>(
      `/api/v1/product-master-data/${record.resourceType}/${record.id}/disable-impact`,
    )
      .then(setImpact)
      .catch((reason: Error) => setError(reason.message))
      .finally(() => setLoading(false));
  }, [disabling, record]);
  async function confirm() {
    setSaving(true);
    setError(null);
    try {
      await request<ProductMasterCommandResult>(
        `/api/v1/product-master-data/${record.resourceType}/${record.id}/status`,
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
      eyebrow="PRODUCT STATUS / IMPACT CHECK"
      onClose={onClose}
    >
      <div className="impact-panel">
        <div className="impact-target">
          <code>{record.code}</code>
          <h3>{record.name}</h3>
          <p>
            {disabling
              ? "停用后不可用于新交易。系统已实时检查商品层级、订单与库存影响。"
              : "启用前将检查所有上级商品定义均处于启用状态。"}
          </p>
        </div>
        {loading && (
          <div className="master-message">正在检查商品、订单与库存影响…</div>
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
                : "请先处理标红的商品或业务影响。"}
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

function fromRecord(record: ProductMasterRecord): FormState {
  return {
    code: record.code,
    name: record.name,
    parentCategoryId: record.parentCategoryId ?? "",
    categoryId: record.categoryId ?? "",
    brandId: record.brandId ?? "",
    baseUomId: record.unitOfMeasureId ?? "",
    productId: record.productId ?? "",
    unitOfMeasureId:
      record.resourceType === "uom_conversion"
        ? (record.unitOfMeasureId ?? "")
        : "",
    barcode: record.barcode ?? "",
    precisionScale: String(record.precisionScale ?? 2),
    allowZeroCost: record.allowZeroCost ?? false,
    factorToBase: record.factorToBase ?? "1",
    usageScope: record.usageScope ?? "both",
  };
}
function labelFor(type: ProductMasterType) {
  return TYPES.find((item) => item.id === type)?.label ?? type;
}
function attribute(item: ProductMasterRecord) {
  if (item.resourceType === "sku") return item.barcode || "条码待维护";
  if (item.resourceType === "product")
    return `${item.brandName ?? "无品牌"} · ${item.unitOfMeasureCode}`;
  if (item.resourceType === "unit_of_measure")
    return `${item.precisionScale ?? 0} 位小数`;
  if (item.resourceType === "uom_conversion")
    return `1 ${item.unitOfMeasureCode} = ${item.factorToBase} 基础单位`;
  if (item.resourceType === "product_category")
    return item.parentCategoryName ?? "顶级分类";
  return "品牌分析口径";
}
function attributeNote(item: ProductMasterRecord) {
  if (item.resourceType === "product")
    return item.allowZeroCost ? "允许零成本例外" : "要求有效库存成本";
  if (item.resourceType === "sku")
    return `${item.productCode} · ${item.unitOfMeasureCode}`;
  if (item.resourceType === "uom_conversion")
    return item.usageScope === "both"
      ? "采购与销售"
      : item.usageScope === "sales"
        ? "仅销售"
        : "仅采购";
  return "编码创建后不可修改";
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
