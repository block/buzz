# Independent `buzz-client` consumer

This small, separate Cargo workspace proves that `buzz-client` can be consumed
without `buzz-cli`, Tauri, workspace-private modules, environment loaders, or
the client's dev-dependencies. It configures a community and asynchronous
signer, lists member channels, and sends one scoped message when a channel is
available.

From the Buzz repository root:

```sh
just buzz-client-consumer-check
```
