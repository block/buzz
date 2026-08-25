# Order profit

The current order view computes net revenue, product cost, gross profit, direct
operating costs, supplier rebate, contribution profit, allocated operating
expense and management operating profit. Revenue is already shipment net sales,
so discount is not subtracted twice. Zero revenue produces a null margin rather
than an invented percentage. Every response exposes `dataAsOf`, quality status,
rule version, source watermark and the non-statutory boundary.
