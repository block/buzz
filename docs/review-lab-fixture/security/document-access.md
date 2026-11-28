# Security fixture: document authorization

Deliberate defect for review testing. The fenced pseudocode is not executable
application code. Authentication middleware supplies `request.auth.accountId`.
Document IDs are globally unique; documents are private to their owning account.

```typescript
async function downloadDocument(request) {
  const document = await db.documents.findUnique({
    where: { id: request.params.documentId },
  });
  if (!document) return response(404);
  return response(200, document.content);
}
```

## Expected review finding

Authentication alone does not authorize access to a document. The lookup ignores
the trusted account identity, so another account's document ID returns its content.
Scope the query to the authenticated account and return a non-disclosing not-found
response when ownership does not match. Do not trust an account ID from the caller.

## Regression scenarios

Use synthetic accounts A and B. Confirm A can read its own document, A cannot read
B's document, and nonexistent IDs return the same not-found response. Verify
unauthenticated requests are rejected by the actual middleware path.
