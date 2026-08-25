# S1 operations runbook

## 日常观察

1. 打开经营驾驶舱，确认期间和币种符合当前经营分析范围。
2. 打开数据质量中心；`blocked` 必须先于经营结论处理，`partial` 必须显示原因。
3. 逐域打开证据端点，记录 trace id、对象 id、差异和最后事实水位。
4. 检查利润投影的 pending events、pending failures、offset 时间和 worker 状态。

## 安全恢复顺序

1. 冻结受影响报表结论，不冻结无关业务命令。
2. 运行对应的范围内 reconciliation，定位权威事实与投影之间的差异。
3. 对投影积压执行幂等 `project_pending`；对完整重建使用已有 rebuild，禁止先删事实。
4. 重新运行 reconciliation，要求差异归零后再恢复 `complete`。
5. 若来源业务单据错误，使用领域冲销命令；不得直接更新数据库事实。

## 告警建议

- 任一对账差异或 pending projection failure：立即告警，状态 `blocked`。
- pending shipment projection events 大于零超过一个 worker 周期：告警，状态 `partial`。
- offset 超过 `PROFIT_DATA_STALE_AFTER_MINUTES`：告警，状态 `partial`。
- 经营驾驶舱普通读取建议 P95 不高于 500ms；管理聚合建议 P95 不高于 2s。
  上线阈值必须在代表性数据量和真实 HTTP 链路下重新确认。

## S1.3 告警处置

1. `RECONCILIATION_DIFFERENCE`：停止引用受影响域的经营结论，按 evidence path 下钻；
   差异归零前不得标记恢复。
2. `PROJECTION_FAILURE`：检查失败摘要和 trace id，修复可重试原因后执行幂等投影；
   不得直接写利润事实。
3. `PROJECTION_BACKLOG`：观察一个 worker 周期；持续存在时检查 worker 日志和 outbox
   最后水位。
4. `PROJECTION_WORKER_DISABLED` / `PROJECTION_STALE`：先确认部署配置、worker 心跳和
   `freshnessAgeSeconds`，不要用刷新页面掩盖过期水位。
5. `SLOW_REPORT_READ`：用 `run.traceId` 查结构化日志，再看 `diagnostics.slowestStage`；
   只有 EXPLAIN 证据表明索引或计划异常时才调整索引。

每次调查记录 route、trace id、duration/target、slowest stage、数据量和最终处置；不得
记录服务凭据、Cookie、CSRF、SQL 绑定值或用户敏感字段。

## S1.4 事件簿处置

1. 运行“扫描当前异常”，将当前范围内的结构化告警创建、重开或标记条件清除。
2. `critical` 在 4 小时、`warning` 在 24 小时内认领；超时事件不得通过重设时限掩盖，
   调整后的时间和操作者会进入追加式轨迹。
3. 依次确认异常、开始处理，并按 evidence path 执行 S1.3 的安全恢复步骤。
4. 修复后必须重新扫描；只有 `conditionStatus=cleared` 才允许标记解决。
5. 异常再次出现时系统重开原事件，保留此前负责人、发生次数和全部审计证据。

处置写权限为 `management_report:manage_incidents`；只读人员仍可查看事件簿，但不能
认领、改时限或解决。

## S1.5 日报、周报与订阅

1. 日报只冻结上一完整自然日；周报只冻结上一完整周（周一至下周一前）。
2. 首份快照只有基线，没有环比；前一周期为零时变化率保持空值。
3. `partial` 或 `blocked` 快照仍作为当时证据保留，但经营结论必须同时展示质量状态。
4. 订阅默认按当前固定 UTC 偏移在 08:00 生成，只写入 Business Dock，不外发。
5. worker 失败时查看 `operating_report_subscription_events` 的 trace 和失败摘要；不得修改
   旧快照或直接补写指标。

订阅写权限为 `management_report:manage_subscriptions`；实际生成还要求订阅所有者保留
`management_report:generate_snapshot`。权限撤销后任务记录失败，并在下一计划周期重试。

## 代表性容量复验

只允许对隔离、空白、名称包含 `s1_capacity` 的临时 PostgreSQL 数据库执行：

```bash
BUSINESS_CORE_S1_CAPACITY_DATABASE_URL='postgres://.../business_s1_capacity_run' \
  cargo test --manifest-path services/business-core/Cargo.toml \
  --test postgres_s1_capacity representative_s1_http_capacity \
  -- --ignored --exact --nocapture
```

测试通过真实 HTTP 路径生成 20,000 销售订单、12,000 出库、10,000 采购订单、
4,999 个 SKU 库位和 24,000 利润事实，并对 dashboard/data-quality 各采样 200 次。
运行者负责在核对数据库精确名称后删除该合成测试库。

## 边界

本手册只处理销售、采购、库存、经营性往来投影和管理利润。不得借恢复操作引入或
模拟发票、银行、凭证、总账、税务或法定报表。
