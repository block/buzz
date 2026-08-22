# Buzz Azure Storage Adapter

This crate is the Azure Blob Storage proof for Buzz's media and git storage
contracts. It intentionally keeps Azure-specific code outside the current S3
paths until the backend passes the required concurrency semantics.

The conformance test covers:

- atomic create-only writes (`If-None-Match: *`),
- ETag compare-and-swap updates (`If-Match`),
- one winner under concurrent create and update races,
- GET body and ETag consistency,
- range reads, streaming reads, HEAD, paginated listing, and idempotent delete,
- bounded multipart file upload with a range-verified large object.

Production clients use Azure's credential environment. On AKS, configure
workload identity with `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, and
`AZURE_FEDERATED_TOKEN_FILE`; no storage account key is required.

## Local validation

Run Azurite's blob service on port 10000, create a container named
`buzz-conformance`, and then run:

```shell
BUZZ_AZURITE_TEST=1 cargo test -p buzz-azure-storage --test azurite_conformance
```

## Private Azure validation

Run the same test from an AKS workload-identity Pod that can resolve the private
Blob endpoint. Scope `Storage Blob Data Contributor` to only the disposable
conformance container, then set:

```shell
BUZZ_AZURE_TEST=1 \
BUZZ_AZURE_STORAGE_ACCOUNT=<account> \
BUZZ_AZURE_CONFORMANCE_CONTAINER=buzz-conformance \
cargo test -p buzz-azure-storage --test azurite_conformance
```

The test uses no account key, writes under a unique `probe/<uuid>` prefix, and
deletes that prefix after a successful run. Version restore and soft-delete
recovery are control-plane validations and remain separate from this data-plane
adapter contract.

Azurite is test-only. Production should use a dedicated Buzz storage account,
private endpoint, private DNS zone, workload identity, soft delete, versioning,
and a lifecycle policy.
