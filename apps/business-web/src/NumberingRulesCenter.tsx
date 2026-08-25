import React from "react";
import { createPortal } from "react-dom";
import {
  type NumberingLedger,
  type NumberingRule,
  type NumberingRuleCommandResult,
  type NumberingRuleList,
  type NumberingSegment,
  request,
} from "./api";
import {
  appendEditableSegment,
  changeEditableScope,
  createEditableSegments,
  moveEditableSegment,
  removeEditableSegment,
  replaceEditableSegment,
} from "./numberingRuleEditorSegments";
import "./numbering-rules.css";

const RECORDS: Record<string, { label: string; group: string; code: string }> =
  {
    sales_order: { label: "销售订单", group: "销售闭环", code: "SO" },
    shipment: { label: "销售出库", group: "销售闭环", code: "SHP" },
    receivable: { label: "经营应收", group: "销售闭环", code: "AR" },
    receipt: { label: "客户收款", group: "销售闭环", code: "RCPT" },
    sales_return: { label: "销售退货", group: "销售闭环", code: "SRET" },
    purchase_order: { label: "采购订单", group: "采购闭环", code: "PO" },
    goods_receipt: { label: "采购入库", group: "采购闭环", code: "GR" },
    payable: { label: "经营应付", group: "采购闭环", code: "AP" },
    supplier_payment: { label: "供应商付款", group: "采购闭环", code: "PAY" },
    purchase_return: { label: "采购退货", group: "采购闭环", code: "PRET" },
    opening: { label: "库存期初", group: "库存经营", code: "OPEN" },
    inventory_count: { label: "库存盘点", group: "库存经营", code: "CNT" },
    purchase_requisition: { label: "采购申请", group: "库存经营", code: "PRQ" },
    profit_adjustment: { label: "经营调整", group: "经营管理", code: "ADJ" },
    management_report: {
      label: "管理报表快照",
      group: "经营管理",
      code: "MGR",
    },
  };

const GROUPS = ["销售闭环", "采购闭环", "库存经营", "经营管理"];

export function NumberingRulesCenter() {
  const [view, setView] = React.useState<"rules" | "ledger">("rules");
  const [data, setData] = React.useState<NumberingRuleList | null>(null);
  const [ledger, setLedger] = React.useState<NumberingLedger | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [ledgerLoading, setLedgerLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [ledgerError, setLedgerError] = React.useState<string | null>(null);
  const [editing, setEditing] = React.useState<NumberingRule | null>(null);

  const load = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setData(await request<NumberingRuleList>("/api/v1/numbering-rules"));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "编码规则加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  const loadLedger = React.useCallback(async () => {
    setLedgerLoading(true);
    setLedgerError(null);
    try {
      setLedger(await request<NumberingLedger>("/api/v1/numbering-ledger"));
    } catch (reason) {
      setLedgerError(
        reason instanceof Error ? reason.message : "编码使用台账加载失败",
      );
    } finally {
      setLedgerLoading(false);
    }
  }, []);

  React.useEffect(() => {
    if (view === "ledger" && !ledger) void loadLedger();
  }, [ledger, loadLedger, view]);

  const activeCount =
    data?.items.filter((item) => item.status === "active").length ?? 0;

  return (
    <section className="page numbering-page">
      <div className="page-head numbering-head">
        <div>
          <p>NUMBERING / CONTROLLED IDENTITY</p>
          <h1>{view === "rules" ? "编码规则" : "编码使用台账"}</h1>
          <span>
            {view === "rules"
              ? "为不同业务记录配置可识别、可追溯的编码；片段从左到右依次组成最终编号。"
              : "查看各主体与期间的序号水位、已提交编号和可解释缺口。台账只读，不提供人工改号。"}
          </span>
        </div>
        <button
          type="button"
          className="numbering-refresh"
          onClick={() => void (view === "rules" ? load() : loadLedger())}
        >
          {view === "rules" ? "刷新规则" : "刷新台账"}
        </button>
      </div>

      <div className="numbering-tabs">
        <button
          type="button"
          className={view === "rules" ? "active" : ""}
          aria-pressed={view === "rules"}
          onClick={() => setView("rules")}
        >
          规则配置
        </button>
        <button
          type="button"
          className={view === "ledger" ? "active" : ""}
          aria-pressed={view === "ledger"}
          onClick={() => setView("ledger")}
        >
          使用台账
        </button>
      </div>

      <div hidden={view !== "rules"}>
        <div className="numbering-summary">
          <div>
            <span>规则总数</span>
            <b>{data?.items.length ?? 0}</b>
          </div>
          <div>
            <span>已启用</span>
            <b>{activeCount}</b>
          </div>
          <div>
            <span>安全回退</span>
            <b>{(data?.items.length ?? 0) - activeCount}</b>
          </div>
          <p>停用后，新记录继续使用服务端默认编码，不会中断业务开单。</p>
        </div>

        {error && <div className="numbering-message error">{error}</div>}
        {loading ? (
          <div className="numbering-message">正在读取编码规则…</div>
        ) : (
          <div className="numbering-groups">
            {GROUPS.map((group) => {
              const rules = (data?.items ?? []).filter(
                (item) => RECORDS[item.recordType]?.group === group,
              );
              return (
                <section className="numbering-group" key={group}>
                  <header>
                    <h2>{group}</h2>
                    <span>{rules.length} 类记录</span>
                  </header>
                  <div className="numbering-grid">
                    {rules.map((rule) => {
                      const record = RECORDS[rule.recordType];
                      return (
                        <article className={rule.status} key={rule.id}>
                          <div className="numbering-card-head">
                            <span>{record?.code ?? rule.recordType}</span>
                            <div>
                              <h3>{record?.label ?? rule.name}</h3>
                              <small>{rule.name}</small>
                            </div>
                            <em>
                              {rule.status === "active" ? "启用" : "回退"}
                            </em>
                          </div>
                          <div className="numbering-policy-tags">
                            <span>{scopeLabel(rule.scopeDimension)}</span>
                            <span>{resetLabel(rule.resetPeriod)}</span>
                          </div>
                          <NumberSegments segments={rule.segments} />
                          <div className="numbering-preview">
                            <span>当前预览</span>
                            <code>{rule.preview}</code>
                          </div>
                          <footer>
                            <small>
                              版本 {rule.version} · {formatTime(rule.updatedAt)}
                            </small>
                            {data?.canManage && (
                              <button
                                type="button"
                                onClick={() => setEditing(rule)}
                              >
                                编辑规则
                              </button>
                            )}
                          </footer>
                        </article>
                      );
                    })}
                  </div>
                </section>
              );
            })}
          </div>
        )}
      </div>

      <div hidden={view !== "ledger"}>
        <NumberingLedgerView
          data={ledger}
          loading={ledgerLoading}
          error={ledgerError}
        />
      </div>

      {editing &&
        createPortal(
          <NumberingRuleEditor
            rule={editing}
            onClose={() => setEditing(null)}
            onSaved={async () => {
              setEditing(null);
              await load();
            }}
          />,
          document.body,
        )}
    </section>
  );
}

function NumberingLedgerView({
  data,
  loading,
  error,
}: {
  data: NumberingLedger | null;
  loading: boolean;
  error: string | null;
}) {
  const [recordType, setRecordType] = React.useState("all");
  if (error) return <div className="numbering-message error">{error}</div>;
  if (loading && !data)
    return <div className="numbering-message">正在读取编码使用台账…</div>;
  if (!data) return null;
  const recordTypes = Array.from(
    new Set([
      ...data.pools.map((item) => item.recordType),
      ...data.recentIssuances.map((item) => item.recordType),
    ]),
  );
  const pools = data.pools.filter(
    (item) => recordType === "all" || item.recordType === recordType,
  );
  const issuances = data.recentIssuances.filter(
    (item) => recordType === "all" || item.recordType === recordType,
  );

  return (
    <div className="numbering-ledger">
      <div className="numbering-ledger-summary">
        <div>
          <span>当前序号池</span>
          <b>{data.summary.poolCount}</b>
          <small>主体 × 重置期间</small>
        </div>
        <div>
          <span>近 30 天发号</span>
          <b>{data.summary.issuedLast30Days}</b>
          <small>仅统计已提交业务记录</small>
        </div>
        <div className={data.summary.gapCount > 0 ? "alert" : "healthy"}>
          <span>检测到缺口</span>
          <b>{data.summary.gapCount}</b>
          <small>
            {data.summary.gapCount > 0 ? "原因见最近发号" : "当前连续"}
          </small>
        </div>
        <div>
          <span>安全回退发号</span>
          <b>{data.summary.fallbackCount}</b>
          <small>规则停用期间累计</small>
        </div>
      </div>

      <div className="numbering-ledger-toolbar">
        <div>
          <b>序号水位</b>
          <span>每个主体和期间独立观察，不跨池比较。</span>
        </div>
        <label>
          <span>记录类型</span>
          <select
            value={recordType}
            onChange={(event) => setRecordType(event.target.value)}
          >
            <option value="all">全部记录</option>
            {recordTypes.map((type) => (
              <option value={type} key={type}>
                {RECORDS[type]?.label ?? type}
              </option>
            ))}
          </select>
        </label>
      </div>

      {pools.length === 0 ? (
        <div className="numbering-message">
          当前筛选下尚未形成序号池；首条业务记录提交后自动建立。
        </div>
      ) : (
        <div className="numbering-pool-grid">
          {pools.map((pool) => (
            <article
              className={pool.gapCount > 0 ? "has-gap" : ""}
              key={`${pool.recordType}:${pool.scopeKey}:${pool.periodKey}`}
            >
              <header>
                <span>{RECORDS[pool.recordType]?.code ?? pool.recordType}</span>
                <div>
                  <b>{RECORDS[pool.recordType]?.label ?? pool.ruleName}</b>
                  <small>
                    {pool.scopeLabel} · {periodLabel(pool.periodKey)}
                  </small>
                </div>
                <em>
                  {pool.gapCount > 0 ? `${pool.gapCount} 个缺口` : "连续"}
                </em>
              </header>
              <div className="numbering-watermark">
                <span>当前水位</span>
                <strong>{pool.currentValue}</strong>
                <i aria-hidden="true" />
              </div>
              <dl>
                <div>
                  <dt>台账内发号</dt>
                  <dd>{pool.issuedCount}</dd>
                </div>
                <div>
                  <dt>最近编号</dt>
                  <dd>{pool.lastNumber ?? "尚无"}</dd>
                </div>
                <div>
                  <dt>最后更新</dt>
                  <dd>{formatTime(pool.lastIssuedAt ?? pool.updatedAt)}</dd>
                </div>
              </dl>
            </article>
          ))}
        </div>
      )}

      <section className="numbering-issuances">
        <header>
          <div>
            <h2>最近发号</h2>
            <p>编号只在业务事务成功提交后进入台账，最多显示最近 100 条。</p>
          </div>
          <span>截至 {formatTime(data.dataAsOf)}</span>
        </header>
        {issuances.length === 0 ? (
          <div className="numbering-message">当前筛选下尚无已提交编号。</div>
        ) : (
          <div className="numbering-ledger-table-wrap">
            <table>
              <thead>
                <tr>
                  <th>业务编号</th>
                  <th>记录</th>
                  <th>主体 / 期间</th>
                  <th>序号</th>
                  <th>来源与连续性</th>
                  <th>发号时间</th>
                </tr>
              </thead>
              <tbody>
                {issuances.map((item) => (
                  <tr
                    className={item.gapBefore > 0 ? "has-gap" : ""}
                    key={item.id}
                  >
                    <td>
                      <code>{item.renderedNumber}</code>
                      <small>{item.aggregateId}</small>
                    </td>
                    <td>
                      {RECORDS[item.recordType]?.label ?? item.recordType}
                    </td>
                    <td>
                      {item.scopeLabel} · {periodLabel(item.periodKey)}
                    </td>
                    <td>#{item.sequenceValue}</td>
                    <td>
                      <span className={`numbering-source ${item.source}`}>
                        {item.source === "governed" ? "受控规则" : "安全回退"}
                      </span>
                      {item.gapBefore > 0 && (
                        <small className="numbering-gap-reason">
                          前置缺口 {item.gapBefore}：{item.gapReason}
                        </small>
                      )}
                    </td>
                    <td>{formatTime(item.issuedAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}

function NumberSegments({ segments }: { segments: NumberingSegment[] }) {
  return (
    <div className="numbering-segments">
      {withSegmentKeys(segments).map(({ segment, key }, index) => (
        <React.Fragment key={key}>
          {index > 0 && <i>＋</i>}
          <span className={segment.type}>{segmentLabel(segment)}</span>
        </React.Fragment>
      ))}
    </div>
  );
}

function NumberingRuleEditor({
  rule,
  onClose,
  onSaved,
}: {
  rule: NumberingRule;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [name, setName] = React.useState(rule.name);
  const [status, setStatus] = React.useState(rule.status);
  const [resetPeriod, setResetPeriod] = React.useState(rule.resetPeriod);
  const [scopeDimension, setScopeDimension] = React.useState(
    rule.scopeDimension,
  );
  const [segmentRows, setSegmentRows] = React.useState(() =>
    createEditableSegments(rule.segments),
  );
  const segments = segmentRows.map((row) => row.segment);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const title = RECORDS[rule.recordType]?.label ?? rule.name;
  const preview = previewNumber(segments, scopeDimension);
  const configurationIssue = numberingConfigurationIssue(
    segments,
    resetPeriod,
    scopeDimension,
  );
  const businessUnitAllowed = !matchesRecord(rule.recordType, [
    "opening",
    "profit_adjustment",
    "management_report",
  ]);
  const legalEntityAllowed = rule.recordType !== "management_report";

  React.useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !saving) onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose, saving]);

  const update = (index: number, segment: NumberingSegment) => {
    setSegmentRows((current) =>
      replaceEditableSegment(current, index, segment),
    );
  };
  const move = (index: number, offset: -1 | 1) => {
    setSegmentRows((current) => moveEditableSegment(current, index, offset));
  };
  const changeScope = (nextScope: NumberingRule["scopeDimension"]) => {
    setScopeDimension(nextScope);
    setSegmentRows((current) => changeEditableScope(current, nextScope));
  };
  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await request<NumberingRuleCommandResult>(
        `/api/v1/numbering-rules/${rule.recordType}`,
        {
          method: "PUT",
          body: JSON.stringify({
            name,
            status,
            resetPeriod,
            scopeDimension,
            segments,
            expectedVersion: rule.version,
          }),
        },
      );
      await onSaved();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "编码规则保存失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="numbering-modal-layer" role="presentation">
      <button
        className="numbering-modal-scrim"
        type="button"
        aria-label="关闭编码规则编辑器"
        onClick={onClose}
      />
      <section
        className="numbering-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="numbering-modal-title"
      >
        <header>
          <div>
            <span>RULE COMPOSER / {RECORDS[rule.recordType]?.code}</span>
            <h2 id="numbering-modal-title">编辑{title}编码</h2>
          </div>
          <button type="button" aria-label="关闭" onClick={onClose}>
            ×
          </button>
        </header>
        <div className="numbering-modal-body">
          <div className="numbering-form-row">
            <label>
              <span>规则名称</span>
              <input
                value={name}
                maxLength={80}
                onChange={(event) => setName(event.target.value)}
              />
            </label>
            <label>
              <span>状态</span>
              <select
                value={status}
                onChange={(event) =>
                  setStatus(event.target.value as NumberingRule["status"])
                }
              >
                <option value="active">启用自定义规则</option>
                <option value="disabled">停用并使用默认规则</option>
              </select>
            </label>
          </div>
          <div className="numbering-policy-row">
            <label>
              <span>序号范围</span>
              <select
                value={scopeDimension}
                onChange={(event) =>
                  changeScope(
                    event.target.value as NumberingRule["scopeDimension"],
                  )
                }
              >
                <option value="global">全局统一序号池</option>
                <option value="legal_entity" disabled={!legalEntityAllowed}>
                  按法定主体独立
                </option>
                <option value="business_unit" disabled={!businessUnitAllowed}>
                  按经营主体独立
                </option>
              </select>
              <small>
                {scopeDimension === "global"
                  ? "所有主体共享连续序号。"
                  : "系统自动加入主体编码片段，避免跨主体重号。"}
              </small>
            </label>
            <label>
              <span>重置周期</span>
              <select
                value={resetPeriod}
                onChange={(event) =>
                  setResetPeriod(
                    event.target.value as NumberingRule["resetPeriod"],
                  )
                }
              >
                <option value="never">永不重置</option>
                <option value="yearly">每年重置</option>
                <option value="monthly">每月重置</option>
                <option value="daily">每日重置</option>
              </select>
              <small>周期切换后，新周期从 1 开始；已生成编号不变。</small>
            </label>
          </div>

          <div className="numbering-compose-head">
            <div>
              <h3>编码片段</h3>
              <p>从上到下决定最终编码的从左到右顺序。</p>
            </div>
            <span>{segments.length} / 8 段</span>
          </div>
          <div className="numbering-compose-list">
            {segmentRows.map(({ segment, key }, index) => (
              <div
                className={`numbering-compose-row ${segment.type}`}
                key={key}
              >
                <b>{String(index + 1).padStart(2, "0")}</b>
                <span className="numbering-kind">
                  {kindLabel(segment.type)}
                </span>
                {segment.type === "fixed" && (
                  <input
                    aria-label={`第 ${index + 1} 段固定字符`}
                    value={segment.value}
                    maxLength={24}
                    onChange={(event) =>
                      update(index, {
                        type: "fixed",
                        value: event.target.value,
                      })
                    }
                  />
                )}
                {segment.type === "date" && (
                  <select
                    aria-label={`第 ${index + 1} 段日期格式`}
                    value={segment.format}
                    onChange={(event) =>
                      update(index, {
                        type: "date",
                        format: event.target.value as Extract<
                          NumberingSegment,
                          { type: "date" }
                        >["format"],
                      })
                    }
                  >
                    <option value="YYYYMMDD">YYYYMMDD</option>
                    <option value="YYYYMM">YYYYMM</option>
                    <option value="YYMMDD">YYMMDD</option>
                    <option value="YYMM">YYMM</option>
                    <option value="YYYY">YYYY</option>
                  </select>
                )}
                {segment.type === "scope" && (
                  <span className="numbering-scope-value">
                    {scopeDimension === "legal_entity"
                      ? "法定主体编码"
                      : "经营主体编码"}
                  </span>
                )}
                {segment.type === "sequence" && (
                  <label className="numbering-width">
                    <input
                      aria-label="序号位数"
                      type="number"
                      min={3}
                      max={10}
                      value={segment.width}
                      onChange={(event) =>
                        update(index, {
                          type: "sequence",
                          width: Number(event.target.value),
                        })
                      }
                    />
                    <span>位</span>
                  </label>
                )}
                <div className="numbering-row-actions">
                  <button
                    type="button"
                    disabled={index === 0}
                    onClick={() => move(index, -1)}
                    aria-label="上移片段"
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    disabled={index === segments.length - 1}
                    onClick={() => move(index, 1)}
                    aria-label="下移片段"
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    disabled={
                      segment.type === "sequence" ||
                      segment.type === "scope" ||
                      segments.length <= 2
                    }
                    onClick={() =>
                      setSegmentRows((current) =>
                        removeEditableSegment(current, index),
                      )
                    }
                    aria-label="删除片段"
                  >
                    ×
                  </button>
                </div>
              </div>
            ))}
          </div>
          <div className="numbering-add">
            <span>添加片段</span>
            <button
              type="button"
              disabled={segments.length >= 8}
              onClick={() =>
                setSegmentRows((current) =>
                  appendEditableSegment(current, {
                    type: "fixed",
                    value: "-",
                  }),
                )
              }
            >
              ＋ 固定字符
            </button>
            <button
              type="button"
              disabled={
                segments.length >= 8 ||
                segments.some((item) => item.type === "date")
              }
              onClick={() =>
                setSegmentRows((current) =>
                  appendEditableSegment(current, {
                    type: "date",
                    format: "YYYYMM",
                  }),
                )
              }
            >
              ＋ 日期
            </button>
          </div>
          <div
            className={`numbering-live-preview ${configurationIssue ? "invalid" : ""}`}
          >
            <span>实时预览 · 示例序号 42</span>
            <code>{preview || "规则尚未完成"}</code>
            <small>
              {configurationIssue ??
                (status === "disabled"
                  ? "当前为停用状态，实际新记录将使用服务端默认格式。"
                  : "保存后，下一条新记录将按此结构生成；历史编码不会改变。")}
            </small>
          </div>
          {error && <div className="numbering-message error">{error}</div>}
        </div>
        <footer>
          <button type="button" className="numbering-cancel" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            disabled={
              saving || !name.trim() || !preview || Boolean(configurationIssue)
            }
            onClick={() => void save()}
          >
            {saving ? "保存中…" : "保存规则"}
          </button>
        </footer>
      </section>
    </div>
  );
}

function withSegmentKeys(segments: NumberingSegment[]) {
  const occurrences = new Map<string, number>();
  return segments.map((segment) => {
    const fingerprint = JSON.stringify(segment);
    const occurrence = (occurrences.get(fingerprint) ?? 0) + 1;
    occurrences.set(fingerprint, occurrence);
    return { segment, key: `${fingerprint}:${occurrence}` };
  });
}

function segmentLabel(segment: NumberingSegment) {
  if (segment.type === "fixed") return segment.value;
  if (segment.type === "date") return segment.format;
  if (segment.type === "scope") return "主体编码";
  return `序号 · ${segment.width} 位`;
}

function kindLabel(type: NumberingSegment["type"]) {
  if (type === "fixed") return "固定字符";
  if (type === "date") return "日期";
  if (type === "scope") return "主体编码";
  return "序号";
}

function previewNumber(
  segments: NumberingSegment[],
  scopeDimension: NumberingRule["scopeDimension"],
) {
  const now = new Date();
  const year = String(now.getFullYear());
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return segments
    .map((segment) => {
      if (segment.type === "fixed")
        return /^[A-Za-z0-9_./-]{1,24}$/.test(segment.value)
          ? segment.value
          : "";
      if (segment.type === "sequence")
        return segment.width >= 3 && segment.width <= 10
          ? "42".padStart(segment.width, "0")
          : "";
      if (segment.type === "scope")
        return scopeDimension === "legal_entity"
          ? "LE01"
          : scopeDimension === "business_unit"
            ? "BU01"
            : "";
      return {
        YYYY: year,
        YYYYMM: `${year}${month}`,
        YYYYMMDD: `${year}${month}${day}`,
        YYMM: `${year.slice(-2)}${month}`,
        YYMMDD: `${year.slice(-2)}${month}${day}`,
      }[segment.format];
    })
    .join("");
}

function scopeLabel(value: NumberingRule["scopeDimension"]) {
  if (value === "legal_entity") return "法定主体独立";
  if (value === "business_unit") return "经营主体独立";
  return "全局序号";
}

function resetLabel(value: NumberingRule["resetPeriod"]) {
  if (value === "daily") return "每日重置";
  if (value === "monthly") return "每月重置";
  if (value === "yearly") return "每年重置";
  return "永不重置";
}

function periodLabel(value: string) {
  if (value === "*") return "永久池";
  if (/^\d{8}$/.test(value))
    return `${value.slice(0, 4)}-${value.slice(4, 6)}-${value.slice(6)}`;
  if (/^\d{6}$/.test(value)) return `${value.slice(0, 4)}-${value.slice(4)}`;
  return value;
}

function matchesRecord(value: string, records: string[]) {
  return records.includes(value);
}

function numberingConfigurationIssue(
  segments: NumberingSegment[],
  resetPeriod: NumberingRule["resetPeriod"],
  scopeDimension: NumberingRule["scopeDimension"],
) {
  const dates = segments.filter(
    (segment): segment is Extract<NumberingSegment, { type: "date" }> =>
      segment.type === "date",
  );
  const scopes = segments.filter((segment) => segment.type === "scope").length;
  if (scopeDimension === "global" && scopes > 0)
    return "全局序号不能包含主体编码片段。";
  if (scopeDimension !== "global" && scopes !== 1)
    return "主体独立序号必须包含一个主体编码片段。";
  if (resetPeriod === "yearly" && dates.length === 0)
    return "每年重置需要至少包含年份的日期片段。";
  if (
    resetPeriod === "monthly" &&
    !dates.some((segment) =>
      ["YYYYMM", "YYYYMMDD", "YYMM", "YYMMDD"].includes(segment.format),
    )
  )
    return "每月重置需要包含年月日期片段。";
  if (
    resetPeriod === "daily" &&
    !dates.some((segment) => ["YYYYMMDD", "YYMMDD"].includes(segment.format))
  )
    return "每日重置需要包含年月日日期片段。";
  return null;
}

function formatTime(value: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}
