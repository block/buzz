# Backend review fixture

This is deliberately flawed sample code for the review lab, not application code.

Requirement: `total` must reject negative quantities and otherwise return
quantity multiplied by price. Review the implementation against that requirement.

```js
function total(quantity, price) {
  if (quantity < 0) return quantity * price;
  return quantity * price;
}
```

Report the failing input and expected behavior in the assigned Buzz thread.
