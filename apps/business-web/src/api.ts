export type Envelope<T> = {
  items: T[];
  dataAsOf: string;
  source:
    | "business-core-b2"
    | "business-core-b3"
    | "business-core-b4"
    | "business-core-s1";
};

export type AgentQueryStage = {
  eventType: string;
  result: "success" | "failure";
  reasonCode: string | null;
  occurredAt: string;
};

export type AgentQueryRun = {
  traceId: string;
  status: "running" | "query_complete" | "complete" | "failed";
  toolName: string | null;
  resultCount: number;
  resourceRefCount: number;
  durationMs: number | null;
  startedAt: string;
  completedAt: string;
  sourceBuzzEventId: string | null;
  responseBuzzEventId: string | null;
  stages: AgentQueryStage[];
};

export type AgentQueryRunList = {
  items: Array<Omit<AgentQueryRun, "sourceBuzzEventId" | "responseBuzzEventId" | "stages">>;
  dataAsOf: string;
};

export type MasterDataRecord = {
  resourceType: string;
  id: string;
  code: string;
  name: string;
  status: string;
  legalEntityId: string | null;
  warehouseId: string | null;
  customerId: string | null;
  supplierId: string | null;
  brandId: string | null;
  businessUnitId: string | null;
  version: number;
};

export type MasterDataList = {
  items: MasterDataRecord[];
  scopeVersion: number;
  effectiveScopeHash: string;
};

export type CoreMasterType =
  | "legal_entity"
  | "business_unit"
  | "customer"
  | "supplier"
  | "warehouse";

export type CoreMasterRecord = {
  resourceType: CoreMasterType;
  id: string;
  code: string;
  name: string;
  status: "active" | "disabled";
  legalEntityId: string | null;
  legalEntityCode: string | null;
  legalEntityName: string | null;
  businessUnitId: string | null;
  businessUnitCode: string | null;
  businessUnitName: string | null;
  countryCode: string | null;
  functionalCurrency: string | null;
  registrationNumber: string | null;
  address: string | null;
  creditCurrency: string | null;
  creditLimitMinor: number | null;
  paymentTermsDays: number | null;
  version: number;
  updatedAt: string;
};

export type CoreMasterList = {
  items: CoreMasterRecord[];
  canManage: boolean;
  dataAsOf: string;
};

export type CoreMasterCommandResult = {
  id: string;
  resourceType: CoreMasterType;
  code: string;
  status: string;
  version: number;
  traceId: string;
  idempotentReplay: boolean;
};

export type CoreMasterDisableImpact = {
  resourceType: CoreMasterType;
  id: string;
  code: string;
  name: string;
  status: string;
  version: number;
  canDisable: boolean;
  impacts: Array<{
    code: string;
    label: string;
    count: number;
    blocking: boolean;
  }>;
  checkedAt: string;
};

export type NumberingSegment =
  | { type: "fixed"; value: string }
  | { type: "date"; format: "YYYY" | "YYYYMM" | "YYYYMMDD" | "YYMM" | "YYMMDD" }
  | { type: "scope" }
  | { type: "sequence"; width: number };

export type NumberingRule = {
  id: string;
  recordType: string;
  name: string;
  segments: NumberingSegment[];
  resetPeriod: "never" | "yearly" | "monthly" | "daily";
  scopeDimension: "global" | "legal_entity" | "business_unit";
  status: "active" | "disabled";
  version: number;
  updatedAt: string;
  preview: string;
};

export type NumberingRuleList = {
  items: NumberingRule[];
  canManage: boolean;
  dataAsOf: string;
};

export type NumberingRuleCommandResult = {
  id: string;
  recordType: string;
  status: string;
  version: number;
  preview: string;
  traceId: string;
  idempotentReplay: boolean;
};

export type NumberingLedger = {
  summary: {
    poolCount: number;
    issuedLast30Days: number;
    gapCount: number;
    fallbackCount: number;
  };
  pools: Array<{
    recordType: string;
    ruleName: string;
    scopeDimension: "global" | "legal_entity" | "business_unit";
    scopeKey: string;
    scopeLabel: string;
    periodKey: string;
    currentValue: number;
    issuedCount: number;
    gapCount: number;
    lastNumber: string | null;
    lastIssuedAt: string | null;
    updatedAt: string;
  }>;
  recentIssuances: Array<{
    id: number;
    recordType: string;
    aggregateId: string;
    renderedNumber: string;
    source: "governed" | "fallback";
    scopeLabel: string;
    periodKey: string;
    sequenceValue: number;
    gapBefore: number;
    gapReason: string | null;
    issuedAt: string;
  }>;
  dataAsOf: string;
};

export type ProductMasterType =
  | "unit_of_measure"
  | "product_category"
  | "brand"
  | "product"
  | "sku"
  | "uom_conversion";

export type ProductMasterRecord = {
  resourceType: ProductMasterType;
  id: string;
  code: string;
  name: string;
  status: "active" | "disabled";
  productId: string | null;
  productCode: string | null;
  productName: string | null;
  categoryId: string | null;
  categoryCode: string | null;
  categoryName: string | null;
  parentCategoryId: string | null;
  parentCategoryCode: string | null;
  parentCategoryName: string | null;
  brandId: string | null;
  brandCode: string | null;
  brandName: string | null;
  unitOfMeasureId: string | null;
  unitOfMeasureCode: string | null;
  unitOfMeasureName: string | null;
  barcode: string | null;
  precisionScale: number | null;
  allowZeroCost: boolean | null;
  factorToBase: string | null;
  usageScope: "sales" | "purchase" | "both" | null;
  version: number;
  updatedAt: string;
};

export type ProductMasterList = {
  items: ProductMasterRecord[];
  canManage: boolean;
  dataAsOf: string;
};

export type ProductMasterCommandResult = {
  id: string;
  resourceType: ProductMasterType;
  code: string;
  status: string;
  version: number;
  traceId: string;
  idempotentReplay: boolean;
};

export type ProductMasterDisableImpact = {
  resourceType: ProductMasterType;
  id: string;
  code: string;
  name: string;
  status: string;
  version: number;
  canDisable: boolean;
  impacts: Array<{
    code: string;
    label: string;
    count: number;
    blocking: boolean;
  }>;
  checkedAt: string;
};

export type OperatingAlert = {
  code: string;
  severity: "warning" | "critical";
  message: string;
  evidencePath: string;
};

export type ReadDiagnostics = {
  status: "healthy" | "slow";
  durationMs: number;
  targetMs: number;
  slowestStage: string | null;
  stages: Array<{ name: string; durationMs: number }>;
};

export type ReportRun = {
  traceId: string;
  status: "completed" | "slow";
  durationMs: number;
  targetMs: number;
  completedAt: string;
};

export type OperatingIncidentEvent = {
  id: string;
  eventType:
    | "detected"
    | "condition_cleared"
    | "reopened"
    | "claimed"
    | "acknowledged"
    | "started"
    | "due_changed"
    | "resolved";
  actorUserId: string;
  actorName: string;
  occurredAt: string;
  traceId: string;
  payload: Record<string, unknown>;
};

export type OperatingIncident = {
  id: string;
  alertCode: string;
  severity: "warning" | "critical";
  message: string;
  evidencePath: string;
  conditionStatus: "active" | "cleared";
  reviewStatus: "open" | "acknowledged" | "in_progress" | "resolved";
  assigneeUserId: string | null;
  assigneeName: string | null;
  dueAt: string;
  overdue: boolean;
  occurrenceCount: number;
  firstSeenAt: string;
  lastSeenAt: string;
  clearedAt: string | null;
  resolvedAt: string | null;
  lastTraceId: string;
  version: number;
  events: OperatingIncidentEvent[];
};

export type OperatingIncidentQueue = Envelope<OperatingIncident>;

export type OperatingTrendMetrics = {
  salesOrderCount: number;
  salesOrderAmount: string;
  shipmentCount: number;
  shippedRevenue: string;
  purchaseOrderCount: number;
  purchaseOrderAmount: string;
  inventoryValueAsOfGeneration: string;
  stockoutCountAsOfGeneration: number;
  managementOperatingProfit: string;
  incidentsOpened: number;
  incidentsResolved: number;
  slaBreached: number;
  averageResolutionHours: string;
};

export type OperatingTrendSnapshot = {
  id: string;
  cadence: "daily" | "weekly";
  periodStart: string;
  periodEnd: string;
  currency: string;
  metrics: OperatingTrendMetrics;
  change: Record<string, string | null> | null;
  dataQualityStatus: "complete" | "partial" | "blocked";
  sourceHash: string;
  generatedAt: string;
  traceId: string;
};

export type OperatingTrendSeries = {
  items: OperatingTrendSnapshot[];
  cadence: "daily" | "weekly";
  currency: string;
  dataAsOf: string;
};

export type OperatingSubscription = {
  id: string;
  cadence: "daily" | "weekly";
  currency: string;
  utcOffsetMinutes: number;
  deliveryHour: number;
  status: "active" | "paused";
  nextRunAt: string;
  lastRunAt: string | null;
  lastSnapshotId: string | null;
  version: number;
};

export type OperatingSubscriptionList = {
  items: OperatingSubscription[];
  dataAsOf: string;
};

export type OperationsDashboard = {
  managementPeriod: string;
  currency: string;
  sales: {
    orderCount: number;
    orderAmount: string;
    committedOrderCount: number;
    shippedOrderCount: number;
    fulfillmentRate: string | null;
    manualHoldCount: number;
    shipmentCount: number;
    shippedRevenue: string;
  };
  purchasing: {
    purchaseOrderCount: number;
    purchaseOrderAmount: string;
    receivedOrderCount: number;
    lineCount: number;
    receivedLineCount: number;
    receiptRate: string | null;
  };
  inventory: {
    skuLocationCount: number;
    stockedLocationCount: number;
    reservedLocationCount: number;
    inventoryValue: string;
    stockoutCount: number;
  };
  profit: {
    netRevenue: string;
    productCost: string;
    grossProfit: string;
    managementOperatingProfit: string;
    managementOperatingMarginRate: string | null;
    sourceWatermark: number;
  };
  reportHealth: {
    status: "complete" | "partial" | "blocked";
    workerEnabled: boolean;
    projectionFresh: boolean;
    freshnessAgeSeconds: number | null;
    staleAfterSeconds: number;
    pendingEvents: number;
    pendingFailures: number;
    updatedAt: string | null;
    alerts: OperatingAlert[];
  };
  diagnostics: ReadDiagnostics;
  run: ReportRun;
  dataAsOf: string;
  warnings: string[];
};

export type DataQuality = {
  status: "complete" | "partial" | "blocked";
  differenceCount: number;
  checks: Array<{
    domain: string;
    status: "consistent" | "difference";
    differenceCount: number;
    evidencePath: string;
  }>;
  projection: {
    workerEnabled: boolean;
    fresh: boolean;
    pendingEvents: number;
    pendingFailures: number;
    freshnessAgeSeconds: number | null;
    staleAfterSeconds: number;
    lastOutboxCreatedAt: string | null;
    lastFactSequence: number | null;
    updatedAt: string | null;
  };
  alerts: OperatingAlert[];
  diagnostics: ReadDiagnostics;
  run: ReportRun;
  dataAsOf: string;
};

export type OrderProfit = {
  salesOrderId: string;
  legalEntityId: string;
  customerId: string;
  currency: string;
  netRevenue: string;
  productCost: string;
  grossProfit: string;
  contributionProfit: string;
  managementOperatingProfit: string;
  managementOperatingMarginRate: string | null;
  dataQualityStatus: string;
  dataAsOf: string;
};

export type ProfitabilityRow = {
  dimensionOne: string;
  dimensionOneId: string | null;
  dimensionTwo?: string | null;
  dimensionTwoId?: string | null;
  currency: string;
  netRevenue: string;
  grossProfit: string;
  contributionProfit: string;
  managementOperatingProfit: string;
  dataQualityStatus: string;
};

export type ProfitAdjustment = {
  id: string;
  adjustmentNumber: string;
  legalEntityId: string;
  managementPeriod: string;
  currency: string;
  status: string;
  version: number;
  lines?: unknown[];
};

export type ManagementReport = {
  reportType: string;
  managementPeriod: string;
  currency: string;
  rows: OrderProfit;
  unallocatedOperatingExpense: string;
  dataQualityStatus: string;
  sourceWatermark: number;
  warnings: string[];
};

export type ManagementSnapshot = {
  id: string;
  snapshotNumber: string;
  reportType: string;
  managementPeriod: string;
  currency: string;
  sourceWatermark: number;
  sourceHash: string;
  status: string;
  version: number;
};

export type SalesOrder = {
  id: string;
  orderNumber: string;
  legalEntityId: string;
  customerId: string;
  currency: string;
  lifecycleStatus: string;
  holdStatus: string;
  fulfillmentStatus: string;
  grossAmount: string;
  orderDate: string;
  updatedAt: string;
  version: number;
};

export type SalesOrderConfirmationLine = {
  skuId: string;
  skuCode: string;
  skuName: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  requiredQuantity: string;
  onHandQuantity: string;
  reservedQuantity: string;
  availableQuantity: string;
  expectedReservedQuantity: string;
  shortageQuantity: string;
};

export type SalesOrderConfirmationPreview = {
  orderId: string;
  orderNumber: string;
  lifecycleStatus: string;
  version: number;
  canConfirm: boolean;
  readiness:
    | "ready"
    | "permission_required"
    | "insufficient_stock"
    | "order_not_draft";
  allAvailable: boolean;
  inventoryAsOf: string;
  lines: SalesOrderConfirmationLine[];
};

export type InventoryBalance = {
  legalEntityId: string;
  warehouseId: string;
  skuId: string;
  onHandQuantity: string;
  reservedQuantity: string;
  quarantinedQuantity: string;
  availableQuantity: string;
  inventoryValue: string;
  averageUnitCost: string | null;
  updatedAt: string;
  version: number;
};

export type InventoryCountOption = {
  legalEntityId: string;
  currency: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  onHandQuantity: string;
  reservedQuantity: string;
  quarantinedQuantity: string;
  inventoryValue: string;
  averageUnitCost: string | null;
};

export type InventoryCountSummary = {
  id: string;
  countNumber: string;
  legalEntityId: string;
  warehouseId: string;
  countDate: string;
  currency: string;
  status: "counting" | "counted" | "posted" | "cancelled";
  lineCount: number;
  varianceLineCount: number;
  varianceValue: string;
  version: number;
  updatedAt: string;
};

export type InventoryCountLine = {
  id: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  snapshotOnHandQuantity: string;
  snapshotReservedQuantity: string;
  snapshotQuarantinedQuantity: string;
  actualOnHandQuantity: string | null;
  snapshotAverageUnitCost: string | null;
  surplusUnitCost: string | null;
  varianceQuantity: string | null;
  varianceValue: string | null;
};

export type InventoryCountDetail = {
  id: string;
  countNumber: string;
  legalEntityId: string;
  warehouseId: string;
  countDate: string;
  currency: string;
  status: string;
  version: number;
  lines: InventoryCountLine[];
};

export type InventoryAgingItem = {
  legalEntityId: string;
  warehouseId: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  onHandQuantity: string;
  reservedQuantity: string;
  quarantinedQuantity: string;
  inventoryValue: string;
  averageUnitCost: string | null;
  currency: string | null;
  lastIssueDate: string | null;
  daysWithoutIssue: number;
  agingBucket: string;
};

export type InventoryTurnover = {
  managementPeriod: string;
  currency: string;
  issuedProductCost: string;
  endingInventoryValue: string;
  turnoverRate: string | null;
  turnoverDays: string | null;
  dataAsOf: string;
  warning: string;
};

export type ReplenishmentSuggestion = {
  id: string;
  legalEntityId: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  preferredSupplierId: string;
  supplierCode: string;
  supplierName: string;
  unitOfMeasureId: string;
  currency: string;
  safetyStock: string;
  reorderPoint: string;
  targetStock: string;
  minimumOrderQuantity: string;
  orderMultiple: string;
  leadTimeDays: number;
  status: "active" | "paused";
  onHandQuantity: string;
  reservedQuantity: string;
  quarantinedQuantity: string;
  availableQuantity: string;
  inboundQuantity: string;
  openRequisitionQuantity: string;
  projectedQuantity: string;
  inventoryValue: string;
  riskState:
    | "critical"
    | "warning"
    | "inbound_covered"
    | "requisition_open"
    | "healthy"
    | "paused";
  suggestedQuantity: string;
  suggestedRequiredDate: string;
  version: number;
  updatedAt: string;
};

export type ReplenishmentInventoryOption = {
  legalEntityId: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  unitOfMeasureId: string;
  availableQuantity: string;
};

export type ReplenishmentSupplierOption = {
  id: string;
  code: string;
  name: string;
};

export type ReplenishmentOptions = {
  inventory: ReplenishmentInventoryOption[];
  suppliers: ReplenishmentSupplierOption[];
  dataAsOf: string;
};

export type PurchaseRequisitionSummary = {
  id: string;
  requisitionNumber: string;
  legalEntityId: string;
  warehouseId: string;
  supplierId: string;
  requestDate: string;
  requiredDate: string;
  currency: string;
  status: "draft" | "confirmed" | "converted" | "cancelled";
  lineCount: number;
  totalQuantity: string;
  version: number;
  updatedAt: string;
};

export type Receivable = {
  id: string;
  receivableNumber: string;
  legalEntityId: string;
  customerId: string;
  salesOrderId: string;
  shipmentId: string;
  currency: string;
  originalAmount: string;
  settledAmount: string;
  openAmount: string;
  dueDate: string;
  status: string;
  isOverdue: boolean;
  overdueDays: number;
  updatedAt: string;
  version: number;
};

export type Receipt = {
  id: string;
  receiptNumber: string;
  legalEntityId: string;
  customerId: string;
  currency: string;
  receiptDate: string;
  amount: string;
  allocatedAmount: string;
  unappliedAmount: string;
  status: string;
  updatedAt: string;
  version: number;
};

export type Shipment = {
  id: string;
  shipmentNumber: string;
  salesOrderId: string;
  warehouseId: string;
  shipmentDate: string;
  status: string;
  confirmedAt: string | null;
  updatedAt: string;
  version: number;
};

export type ShipmentDraftOptionLine = {
  orderId: string;
  orderNumber: string;
  customerCode: string;
  customerName: string;
  currency: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  salesOrderLineId: string;
  lineNumber: number;
  skuId: string;
  skuCode: string;
  skuName: string;
  orderedQuantity: string;
  shippedQuantity: string;
  reservationOpenQuantity: string;
  draftAllocatedQuantity: string;
  shippableQuantity: string;
};

export type ShipmentDraftOptions = {
  canCreate: boolean;
  dataAsOf: string;
  items: ShipmentDraftOptionLine[];
};

export type ShipmentConfirmationLine = {
  salesOrderLineId: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  quantity: string;
  reservationOpenQuantity: string;
  onHandQuantity: string;
  reservedQuantity: string;
  averageUnitCost: string | null;
  expectedCostAmount: string | null;
  ready: boolean;
  readiness: "ready" | "missing_inventory_cost" | "insufficient_inventory";
};

export type ShipmentConfirmationPreview = {
  shipmentId: string;
  shipmentNumber: string;
  salesOrderId: string;
  orderNumber: string;
  customerCode: string;
  customerName: string;
  warehouseCode: string;
  warehouseName: string;
  shipmentDate: string;
  status: string;
  version: number;
  currency: string;
  salesAmount: string;
  expectedCostAmount: string | null;
  expectedReceivableAmount: string;
  expectedDueDate: string;
  canConfirm: boolean;
  readiness:
    | "ready"
    | "permission_required"
    | "shipment_not_draft"
    | "order_on_hold"
    | "order_not_fulfillable"
    | "missing_inventory_cost"
    | "insufficient_inventory";
  inventoryAsOf: string;
  lines: ShipmentConfirmationLine[];
};

export type InventoryOpening = {
  id: string;
  batchNumber: string;
  legalEntityId: string;
  businessDate: string;
  currency: string;
  status: string;
  postedAt: string | null;
  reversedAt: string | null;
  version: number;
};

export type InventoryMovement = {
  id: string;
  legalEntityId: string;
  warehouseId: string;
  skuId: string;
  movementType: string;
  quantity: string;
  unitCost: string;
  totalCost: string;
  businessDate: string;
  postedAt: string;
};

export type PurchaseOrder = {
  id: string;
  purchaseOrderNumber: string;
  legalEntityId: string;
  supplierId: string;
  currency: string;
  lifecycleStatus: string;
  receivingStatus: string;
  grossAmount: string;
  orderDate: string;
  updatedAt: string;
  version: number;
};

export type PurchaseDelivery = {
  purchaseOrderId: string;
  purchaseOrderNumber: string;
  legalEntityId: string;
  supplierId: string;
  supplierCode: string;
  supplierName: string;
  buyerUserId: string;
  orderDate: string;
  expectedDeliveryDate: string | null;
  promisedDeliveryDate: string | null;
  commitmentSource: "planned" | "supplier_commitment";
  commitmentId: string | null;
  commitmentRevision: number;
  commitmentNote: string | null;
  commitmentRecordedAt: string | null;
  lifecycleStatus: string;
  receivingStatus: string;
  currency: string;
  grossAmount: string;
  orderedQuantity: string;
  receivedQuantity: string;
  cancelledQuantity: string;
  openQuantity: string;
  receiptCount: number;
  firstReceiptDate: string | null;
  lastReceiptDate: string | null;
  deliveryStatus:
    | "cancelled"
    | "unscheduled"
    | "completed_on_time"
    | "completed_late"
    | "overdue"
    | "due_today"
    | "due_soon"
    | "on_track";
  deliveryVarianceDays: number | null;
  updatedAt: string;
  version: number;
};

export type PurchaseDeliveryResponse = {
  items: PurchaseDelivery[];
  canManageCommitments: boolean;
  dataAsOf: string;
};

export type SupplierDeliveryPerformance = {
  supplierId: string;
  supplierCode: string;
  supplierName: string;
  orderCount: number;
  openOrderCount: number;
  overdueOrderCount: number;
  completedOrderCount: number;
  onTimeOrderCount: number;
  onTimeRate: string | null;
  orderedQuantity: string;
  receivedQuantity: string;
  fulfillmentRate: string | null;
  returnedQuantity: string;
  qualityAcceptanceRate: string | null;
};

export type SupplierPerformanceResponse = {
  items: SupplierDeliveryPerformance[];
  periodStart: string;
  periodEnd: string;
  dataAsOf: string;
};

export type PurchaseOrderDraftLine = {
  skuId: string;
  warehouseId: string;
  unitOfMeasureId: string;
  quantity: string;
  unitPrice: string;
  discountAmount: string;
  taxRate: string;
};

export type PurchaseOrderDraft = {
  id: string;
  purchaseOrderNumber: string;
  legalEntityId: string;
  supplierId: string;
  businessUnitId: string;
  currency: string;
  orderDate: string;
  expectedDeliveryDate: string | null;
  paymentTermsDays: number;
  supplierReference: string | null;
  businessNote: string | null;
  lifecycleStatus: string;
  version: number;
  lines: PurchaseOrderDraftLine[];
};

export type PurchaseOrderEntryOptions = {
  canCreate: boolean;
  canUpdate: boolean;
  dataAsOf: string;
  draft: PurchaseOrderDraft | null;
};

export type PurchaseOrderConfirmationLine = {
  lineNumber: number;
  skuCode: string;
  skuName: string;
  warehouseCode: string;
  warehouseName: string;
  unitCode: string;
  unitName: string;
  orderedQuantity: string;
  unitPrice: string;
  discountAmount: string;
  netAmount: string;
  taxRate: string;
  taxAmount: string;
  grossAmount: string;
  ready: boolean;
  readiness: "ready" | "master_data_inactive";
};

export type PurchaseOrderConfirmationPreview = {
  orderId: string;
  orderNumber: string;
  supplierCode: string;
  supplierName: string;
  currency: string;
  orderDate: string;
  expectedDeliveryDate: string | null;
  paymentTermsDays: number;
  lifecycleStatus: string;
  version: number;
  subtotalAmount: string;
  discountAmount: string;
  netAmount: string;
  taxAmount: string;
  grossAmount: string;
  warehouseCount: number;
  canConfirm: boolean;
  readiness:
    | "ready"
    | "permission_required"
    | "order_not_draft"
    | "supplier_inactive"
    | "line_incomplete";
  checkedAt: string;
  lines: PurchaseOrderConfirmationLine[];
};

export type GoodsReceipt = {
  id: string;
  goodsReceiptNumber: string;
  purchaseOrderId: string;
  legalEntityId: string;
  supplierId: string;
  warehouseId: string;
  receiptDate: string;
  status: string;
  currency: string;
  grossAmount: string;
  inventoryCostAmount: string;
  updatedAt: string;
  version: number;
};

export type GoodsReceiptDraftOptionLine = {
  orderId: string;
  orderNumber: string;
  supplierCode: string;
  supplierName: string;
  currency: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  purchaseOrderLineId: string;
  lineNumber: number;
  skuId: string;
  skuCode: string;
  skuName: string;
  unitCode: string;
  unitName: string;
  orderedQuantity: string;
  receivedQuantity: string;
  cancelledQuantity: string;
  draftAllocatedQuantity: string;
  receivableQuantity: string;
};

export type GoodsReceiptDraftOptions = {
  canCreate: boolean;
  dataAsOf: string;
  items: GoodsReceiptDraftOptionLine[];
};

export type GoodsReceiptConfirmationLine = {
  purchaseOrderLineId: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  receivedQuantity: string;
  orderRemainingQuantity: string;
  provisionalUnitCost: string;
  provisionalInventoryCost: string;
  currentOnHandQuantity: string;
  currentInventoryValue: string;
  currentAverageUnitCost: string | null;
  projectedOnHandQuantity: string;
  projectedInventoryValue: string;
  projectedAverageUnitCost: string;
  ready: boolean;
  readiness: "ready" | "order_not_open" | "over_receipt";
};

export type GoodsReceiptConfirmationPreview = {
  receiptId: string;
  receiptNumber: string;
  purchaseOrderId: string;
  orderNumber: string;
  supplierCode: string;
  supplierName: string;
  warehouseCode: string;
  warehouseName: string;
  receiptDate: string;
  status: string;
  version: number;
  currency: string;
  expectedInventoryCost: string;
  expectedTaxAmount: string;
  expectedPayableAmount: string;
  expectedDueDate: string;
  canConfirm: boolean;
  readiness:
    | "ready"
    | "permission_required"
    | "receipt_not_draft"
    | "order_not_open"
    | "over_receipt";
  inventoryAsOf: string;
  lines: GoodsReceiptConfirmationLine[];
};

export type Payable = {
  id: string;
  payableNumber: string;
  legalEntityId: string;
  supplierId: string;
  purchaseOrderId: string;
  goodsReceiptId: string;
  currency: string;
  originalAmount: string;
  settledAmount: string;
  openAmount: string;
  dueDate: string;
  status: string;
  isOverdue: boolean;
  overdueDays: number;
  updatedAt: string;
  version: number;
};

export type SupplierPayment = {
  id: string;
  supplierPaymentNumber: string;
  legalEntityId: string;
  supplierId: string;
  currency: string;
  paymentDate: string;
  amount: string;
  allocatedAmount: string;
  unappliedAmount: string;
  status: string;
  updatedAt: string;
  version: number;
};

export type BusinessReturn = {
  id: string;
  returnNumber: string;
  sourceId: string;
  orderId: string;
  partnerId: string;
  warehouseId: string;
  returnDate: string;
  currency: string;
  reasonCode: string;
  amount: string;
  status: string;
  workflowStatus: string;
  version: number;
  updatedAt: string;
};

export type ReturnOptionLine = {
  sourceId: string;
  sourceNumber: string;
  orderId: string;
  orderNumber: string;
  partnerId: string;
  partnerCode: string;
  partnerName: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  currency: string;
  sourceLineId: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  sourceQuantity: string;
  returnedQuantity: string;
  returnableQuantity: string;
};

export type ReturnOptions = {
  canCreate: boolean;
  dataAsOf: string;
  items: ReturnOptionLine[];
};

export type ReturnInspectionLine = {
  returnLineId: string;
  skuId: string;
  skuCode: string;
  skuName: string;
  quantity: string;
  unitCost: string;
};

export type ReturnInspection = {
  returnId: string;
  returnNumber: string;
  version: number;
  inspectionStatus: string;
  lines: ReturnInspectionLine[];
};

export type ReturnMetric = {
  legalEntityId: string;
  currency: string;
  managementPeriod: string;
  shippedSalesAmount: string;
  salesReturnCount: number;
  salesReturnAmount: string;
  salesReturnRate: string | null;
  returnLossAmount: string;
  scrapCostAmount: string;
  receivedPurchaseAmount: string;
  purchaseReturnCount: number;
  purchaseReturnAmount: string;
  purchaseReturnRate: string | null;
};

export type ReturnAnalytics = {
  items: ReturnMetric[];
  managementPeriod: string;
  currency: string;
  dataAsOf: string;
  warnings: string[];
};

async function csrfToken(): Promise<string | null> {
  const response = await fetch("/api/session", {
    credentials: "include",
    headers: { accept: "application/json" },
  });
  if (!response.ok) return null;
  const body = (await response.json()) as { csrfToken?: unknown };
  return typeof body.csrfToken === "string" && body.csrfToken.length > 0
    ? body.csrfToken
    : null;
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const method = init?.method ?? "GET";
  const headers = new Headers(init?.headers);
  if (method !== "GET" && method !== "HEAD") {
    const csrf = await csrfToken();
    if (!csrf) throw new Error("登录会话缺少 CSRF 凭据，请重新登录");
    headers.set("content-type", "application/json");
    headers.set("x-csrf-token", csrf);
    headers.set("idempotency-key", crypto.randomUUID());
  }
  const response = await fetch(path, {
    ...init,
    method,
    headers,
    credentials: "include",
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(apiErrorMessage(body, response.status));
  }
  return body as T;
}

export function apiErrorMessage(body: unknown, status: number): string {
  if (body && typeof body === "object") {
    const error = body as { code?: unknown; message?: unknown };
    if (error.code === "not_found_or_forbidden") {
      return "资源不存在，或当前账号无权访问";
    }
    if (typeof error.message === "string" && error.message.length > 0) {
      return error.message;
    }
    if (typeof error.code === "string" && error.code.length > 0) {
      return error.code;
    }
  }
  return `请求失败（${status}）`;
}
