# UI fixture: save button state

Deliberate defect for review testing. The fenced sample is illustrative React code,
not application code. A valid form should be submittable while idle and blocked
while a save is in progress.

```tsx
function SaveButton({ saving, valid, onSave }) {
  return (
    <button disabled={!saving || !valid} onClick={onSave}>
      {saving ? "Saving..." : "Save"}
    </button>
  );
}
```

## Expected review finding

For a valid form, `!saving` disables the idle button and enables it during a save.
Use `saving || !valid` and guard duplicate submissions in the save handler. Check
both idle and saving states with valid and invalid inputs.
