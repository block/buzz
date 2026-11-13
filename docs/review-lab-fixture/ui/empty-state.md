# UI review fixture

This is deliberately flawed pseudocode for the review lab, not application code.

Requirement: show “Loading…” while a request is pending, “No results” after a
successful empty response, and the results list otherwise.

```jsx
function Results({ loading, items }) {
  if (items.length === 0) return <p>No results</p>;
  if (loading) return <p>Loading…</p>;
  return <ul>{items.map(item => <li key={item.id}>{item.name}</li>)}</ul>;
}
```

Review the ordering for an initial request with `loading=true` and `items=[]`.
Report the user-visible behavior in the assigned Buzz thread.
