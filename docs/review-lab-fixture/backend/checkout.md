# Backend fixture: checkout quantity

Deliberate defect for review testing. The following fenced sample is not executable
application code. Quantities must be integers between 1 and 100 inclusive.

```typescript
function checkoutTotal(unitPriceCents: number, quantity: number): number {
  if (quantity > 100) throw new Error("Too many items");
  return unitPriceCents * quantity;
}
```

## Expected review finding

Zero, negative, fractional, and non-finite quantities are accepted. For example,
`checkoutTotal(500, -2)` returns `-1000`. Validate a finite integer in the full
allowed range; exercise 0, -2, 1.5, NaN, and the valid boundaries 1 and 100.
