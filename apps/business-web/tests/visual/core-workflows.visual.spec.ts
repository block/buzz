import { expect, type Locator, type Page, test } from "@playwright/test";

const ZOOMS = [80, 100, 125] as const;
const FIXED_NOW = "2026-08-23T09:30:00+08:00";

const envelope = (items: unknown[]) => ({
  items,
  dataAsOf: "2026-08-23T01:30:00Z",
  source: "business-core-s1",
});

const salesOrder = {
  id: "sales-order-1",
  orderNumber: "SO-202608-000001",
  legalEntityId: "legal-entity-1",
  customerId: "customer-1",
  currency: "CNY",
  lifecycleStatus: "confirmed",
  holdStatus: "none",
  fulfillmentStatus: "reserved",
  grossAmount: "1234567.89",
  orderDate: "2026-08-23",
  updatedAt: "2026-08-23T01:30:00Z",
  version: 2,
};

const purchaseOrder = {
  id: "purchase-order-1",
  purchaseOrderNumber: "PO-202608-000001",
  legalEntityId: "legal-entity-1",
  supplierId: "supplier-1",
  currency: "CNY",
  lifecycleStatus: "confirmed",
  receivingStatus: "unreceived",
  grossAmount: "2345678.90",
  orderDate: "2026-08-23",
  updatedAt: "2026-08-23T01:30:00Z",
  version: 2,
};

const inventoryBalance = {
  legalEntityId: "legal-entity-1",
  warehouseId: "warehouse-1",
  skuId: "sku-1",
  onHandQuantity: "15000",
  reservedQuantity: "2000",
  quarantinedQuantity: "0",
  availableQuantity: "13000",
  inventoryValue: "6000000",
  averageUnitCost: "400",
  updatedAt: "2026-08-23T01:30:00Z",
  version: 1,
};

const inventoryMovement = {
  id: "movement-1",
  legalEntityId: "legal-entity-1",
  warehouseId: "warehouse-1",
  skuId: "sku-1",
  movementType: "purchase_receipt",
  quantity: "15000",
  unitCost: "400",
  totalCost: "6000000",
  businessDate: "2026-08-23",
  postedAt: "2026-08-23T01:30:00Z",
};

function masterRecord(resource: string) {
  const values: Record<string, { id: string; code: string; name: string }> = {
    legal_entity: { id: "legal-entity-1", code: "LE-01", name: "上海经营主体" },
    customer: { id: "customer-1", code: "CUS-01", name: "华东重点客户" },
    supplier: { id: "supplier-1", code: "SUP-01", name: "核心供应商" },
    business_unit: { id: "business-unit-1", code: "BU-01", name: "商贸业务部" },
    sku: { id: "sku-1", code: "SKU-001", name: "标准测试商品" },
    warehouse: { id: "warehouse-1", code: "WH-01", name: "上海中心仓" },
    unit_of_measure: { id: "uom-1", code: "PCS", name: "件" },
  };
  const value = values[resource];
  if (!value) return [];
  return [{
    resourceType: resource,
    ...value,
    status: "active",
    legalEntityId: resource === "legal_entity" ? null : "legal-entity-1",
    warehouseId: null,
    customerId: null,
    supplierId: null,
    brandId: null,
    businessUnitId: null,
    version: 1,
  }];
}

async function installBusinessFixtures(page: Page, zoom: number) {
  await page.clock.setFixedTime(new Date(FIXED_NOW));
  await page.addInitScript(
    ({ value }) => localStorage.setItem("bizfin.business.pageZoom", String(value)),
    { value: zoom / 100 },
  );
  await page.addInitScript(() => {
    if (localStorage.getItem("bizfin.business.navigationCollapsed") === null) {
      localStorage.setItem("bizfin.business.navigationCollapsed", "false");
    }
  });
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    let body: unknown = envelope([]);

    if (path === "/api/session") {
      body = { authenticated: true, subject: "visual-user", displayName: "视觉巡检", csrfToken: "visual-csrf" };
    } else if (path === "/api/v1/sales-orders") {
      body = envelope([salesOrder]);
    } else if (path === "/api/v1/purchase-orders") {
      body = envelope([purchaseOrder]);
    } else if (path === "/api/v1/purchase-orders/entry-options") {
      body = { canCreate: true, canUpdate: true, dataAsOf: "2026-08-23T01:30:00Z", draft: null };
    } else if (path.startsWith("/api/v1/master-data/")) {
      body = {
        items: masterRecord(path.split("/").at(-1) ?? ""),
        scopeVersion: 1,
        effectiveScopeHash: "visual-fixture",
      };
    } else if (path === "/api/v1/inventory-balances") {
      body = envelope([inventoryBalance]);
    } else if (path === "/api/v1/inventory-movements") {
      body = envelope([inventoryMovement]);
    } else if (path === "/api/v1/inventory-openings") {
      body = envelope([]);
    } else if (path === "/api/v1/inventory-counts/options") {
      body = { items: [{
        legalEntityId: "legal-entity-1",
        currency: "CNY",
        warehouseId: "warehouse-1",
        warehouseCode: "WH-01",
        warehouseName: "上海中心仓",
        skuId: "sku-1",
        skuCode: "SKU-001",
        skuName: "标准测试商品",
        onHandQuantity: "15000",
        reservedQuantity: "2000",
        quarantinedQuantity: "0",
        inventoryValue: "6000000",
        averageUnitCost: "400",
      }] };
    } else if (path === "/api/v1/inventory-counts") {
      body = envelope([]);
    } else if (path === "/api/v1/inventory-aging") {
      body = { items: [] };
    } else if (path === "/api/v1/inventory-turnover") {
      body = {
        managementPeriod: "2026-08",
        currency: "CNY",
        issuedProductCost: "4000000",
        endingInventoryValue: "6000000",
        turnoverRate: "0.67",
        turnoverDays: "46.50",
        dataAsOf: "2026-08-23T01:30:00Z",
        warning: "",
      };
    }

    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });
  });
}

async function expectNoHorizontalOverflow(locator: Locator) {
  const dimensions = await locator.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(dimensions.scrollWidth, `horizontal overflow: ${JSON.stringify(dimensions)}`).toBeLessThanOrEqual(dimensions.clientWidth + 1);
}

async function expectNoTextClipping(locator: Locator) {
  const dimensions = await locator.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(dimensions.scrollWidth, `clipped text: ${JSON.stringify(dimensions)}`).toBeLessThanOrEqual(dimensions.clientWidth + 1);
}

async function expectSingleLine(locator: Locator) {
  await expect(locator).toBeVisible();
  const lineTops = await locator.evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    return [...range.getClientRects()].reduce<number[]>((tops, rect) => {
      if (!tops.some((top) => Math.abs(top - rect.top) < 1)) tops.push(rect.top);
      return tops;
    }, []);
  });
  expect(lineTops, `wrapped text occupies ${lineTops.length} lines`).toHaveLength(1);
}

async function expectDialogInsideViewport(page: Page, dialog: Locator) {
  await expect(dialog).toBeVisible();
  const box = await dialog.boundingBox();
  const viewport = page.viewportSize();
  expect(box).not.toBeNull();
  expect(viewport).not.toBeNull();
  if (!box || !viewport) return;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 1);
  expect(box.y + box.height).toBeLessThanOrEqual(viewport.height + 1);
}

async function expectIndentedNavigation(page: Page, activeLabel: string) {
  const navigation = page.getByRole("navigation", { name: "业务导航" });
  await expect(navigation.locator(".rail-group")).toHaveCount(4);
  await expect(navigation.locator(".rail-group-head strong")).toHaveText([
    "经营控制",
    "基础资料",
    "业务闭环",
    "经营分析",
  ]);
  const groupLabel = navigation.locator(".rail-group-head strong").first();
  const childLabel = navigation.locator(".rail-item-label").first();
  const [groupBox, childBox] = await Promise.all([
    groupLabel.boundingBox(),
    childLabel.boundingBox(),
  ]);
  expect(groupBox).not.toBeNull();
  expect(childBox).not.toBeNull();
  if (groupBox && childBox) expect(childBox.x).toBeGreaterThan(groupBox.x + 12);
  await expect(navigation.getByRole("link", { name: new RegExp(activeLabel) })).toHaveClass(/active/);
}

async function openPage(page: Page, section: "sales" | "purchasing" | "inventory", zoom: number) {
  await installBusinessFixtures(page, zoom);
  await page.goto(`/#${section}`);
  await expect(page.getByRole("button", { name: `当前缩放 ${zoom}%` })).toBeVisible();
}

for (const zoom of ZOOMS) {
  test(`销售闭环页面与新增弹窗在 ${zoom}% 下稳定`, async ({ page }) => {
    await openPage(page, "sales", zoom);
    await expect(page.getByRole("heading", { name: "销售订单闭环" })).toBeVisible();
    await expectIndentedNavigation(page, "销售订单闭环");
    await expectNoHorizontalOverflow(page.locator("main"));
    await expectSingleLine(page.locator(".money-cell strong").first());
    await expect(page).toHaveScreenshot(`sales-page-${zoom}.png`);

    await page.getByRole("button", { name: "新增销售订单" }).click();
    const dialog = page.getByRole("dialog", { name: "新增销售订单" });
    await expect(dialog.getByRole("heading", { name: "录入销售订单" })).toBeVisible();
    await expectDialogInsideViewport(page, dialog);
    await expectNoHorizontalOverflow(dialog);
    await expect(page).toHaveScreenshot(`sales-modal-${zoom}.png`);
  });

  test(`采购闭环页面与新增弹窗在 ${zoom}% 下稳定`, async ({ page }) => {
    await openPage(page, "purchasing", zoom);
    await expect(page.getByRole("heading", { name: "采购订单闭环" })).toBeVisible();
    await expectNoHorizontalOverflow(page.locator("main"));
    await expectSingleLine(page.locator(".money-cell strong").first());
    await expect(page).toHaveScreenshot(`purchase-page-${zoom}.png`);

    await page.getByRole("button", { name: "新增采购订单" }).click();
    const dialog = page.getByRole("dialog", { name: "新增采购订单" });
    await expect(dialog.getByRole("heading", { name: "录入采购订单" })).toBeVisible();
    await expectDialogInsideViewport(page, dialog);
    await expectNoHorizontalOverflow(dialog);
    await expect(page).toHaveScreenshot(`purchase-modal-${zoom}.png`);
  });

  test(`库存台账页面与盘点弹窗在 ${zoom}% 下稳定`, async ({ page }) => {
    await openPage(page, "inventory", zoom);
    await expect(page.getByRole("heading", { name: "库存台账" })).toBeVisible();
    await expect(page.locator(".balance-table tbody tr")).toHaveCount(1);
    await expectNoHorizontalOverflow(page.locator("main"));
    for (const value of await page.locator(".inventory-equation strong").all()) {
      await expectSingleLine(value);
      await expectNoTextClipping(value);
    }
    for (const cell of await page.locator(".balance-table tbody td:not(:first-child)").all()) await expectSingleLine(cell);
    await expect(page).toHaveScreenshot(`inventory-page-${zoom}.png`);

    await page.getByRole("button", { name: "期初与盘点" }).click();
    await page.getByRole("button", { name: "新建盘点任务" }).click();
    const dialog = page.getByRole("dialog", { name: "新建库存盘点" });
    await expect(dialog.getByText("创建即冻结库存范围")).toBeVisible();
    await expectDialogInsideViewport(page, dialog);
    await expectNoHorizontalOverflow(dialog);
    await expect(page).toHaveScreenshot(`inventory-modal-${zoom}.png`);
  });
}

test("左侧导航可以隐藏、恢复并记忆选择", async ({ page }) => {
  await openPage(page, "sales", 100);
  const navigation = page.getByRole("navigation", { name: "业务导航" });
  await expect(navigation).toBeVisible();

  await page.getByRole("button", { name: "隐藏导航栏" }).click();
  await expect(navigation).toBeHidden();
  await expect(page.getByRole("button", { name: "显示导航栏" })).toBeVisible();
  await expectNoHorizontalOverflow(page.locator("main"));
  await expect(page).toHaveScreenshot("navigation-hidden-100.png");

  await page.reload();
  await expect(page.getByRole("button", { name: "显示导航栏" })).toBeVisible();
  await page.getByRole("button", { name: "显示导航栏" }).click();
  await expect(navigation).toBeVisible();

  await page.reload();
  await expect(navigation).toBeVisible();
});
