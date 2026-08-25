# 商品主数据中心

商品主数据中心维护六类权威对象：商品分类、品牌、计量单位、商品（SPU）、SKU/条码和商品单位换算。它们沿“分类/品牌/单位 → 商品 → SKU/换算”关系组织，为销售、采购、库存和经营报表提供同一商品语义。

## 维护规则

- 读取要求 `business_product_master:read`，写入要求 `business_product_master:manage`。
- 编码、父级分类、商品所属分类/品牌/基础单位、SKU 所属商品以及换算的商品/单位在创建后不可修改。
- 商品名称、零成本例外策略、SKU 名称/条码和换算因子可以通过带 `expectedVersion` 的受控修订更新。
- 条码全局唯一；换算在同一商品和单位组合内唯一。换算单位不能等于商品基础单位，换算因子表示“一换算单位折合多少基础单位”。
- 新品牌自动向创建者授予品牌范围；品牌关联的商品、SKU 和换算继续执行品牌范围过滤。
- 所有命令使用幂等键、对象事务锁、乐观版本、审计事件和 Outbox。

## 停用影响

停用分类前检查启用中的子分类和商品；停用品牌前检查启用商品和未完成订单；停用单位前检查基础单位商品和启用换算；停用商品或 SKU 前检查启用下级、库存余额及未完成销售/采购行。停用预览仅用于说明，确认命令会再次读取实时业务事实。

当前交易单据继续以商品基础单位入账。换算规则作为受控商品定义提供，后续接入交易录入时必须把选择单位、换算因子快照和基础数量同时保存在订单行，不能回溯套用最新换算。

## 接口

- `GET /api/v1/product-master-data`
- `POST /api/v1/product-master-data`
- `PUT /api/v1/product-master-data/{resourceType}/{id}`
- `GET /api/v1/product-master-data/{resourceType}/{id}/disable-impact`
- `POST /api/v1/product-master-data/{resourceType}/{id}/status`
