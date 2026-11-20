# Backend fixture: account-scoped invoice lookup

Deliberate defect for review testing. This fenced pseudocode is not executable
application code. Invoice identifiers are globally unique, but users must only
read invoices belonging to their authenticated account.

```typescript
async function getInvoice(authenticatedAccountId: string, invoiceId: string) {
  return db.invoices.findUnique({ where: { id: invoiceId } });
}
```

## Expected review finding

The lookup ignores the authenticated account. A caller who supplies another
account's invoice ID can receive its invoice. Scope the lookup to the trusted
account and return a non-disclosing not-found response for cross-account access.
Test with two accounts and confirm neither can retrieve the other's invoice.
