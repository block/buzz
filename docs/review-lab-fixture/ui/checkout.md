# UI fixture: product search ordering

Deliberate defect for review testing. These illustrative functions represent a
search field where only the latest query's results should be displayed.

```typescript
async function onQueryChange(query: string) {
  const results = await searchProducts(query);
  setResults(results);
}
```

## Expected review finding

Typing `ca` then `cake` can show `ca` results last if the older request resolves
after the newer one. Track the current request or cancel obsolete requests and
ignore stale completions. Test with controlled responses resolved out of order.
