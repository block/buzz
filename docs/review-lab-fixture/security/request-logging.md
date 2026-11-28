# Security fixture: request logging

Deliberate defect for review testing. This fenced pseudocode is not executable
application code. Login requests contain passwords in the body; authenticated
requests can carry authorization headers and session cookies.

```typescript
function logRequest(request) {
  logger.info({
    path: request.path,
    headers: request.headers,
    body: request.body,
  }, "Incoming request");
}
```

## Expected review finding

Unfiltered headers and bodies can persist passwords, bearer tokens, and cookies
in logs. Log only approved metadata, such as method, route template, and request
ID. Omit authentication material and sensitive payloads rather than relying on
callers to remember redaction.

## Regression scenarios

Send synthetic credential markers through a login body, Authorization header, and
Cookie header. Capture the logger output and assert none of the markers appear,
while the permitted request metadata remains available. Use fake markers only.
