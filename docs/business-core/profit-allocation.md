# Profit allocation

Direct, net-revenue, product-cost, shipped-quantity and fixed-weight bases use a
single deterministic allocator. It rounds to currency cents, then assigns the
largest remainders in descending order with sales-order UUID as the stable tie
break. Allocated plus unallocated always equals the requested amount. LLM output
is never accepted as an allocation calculation.
