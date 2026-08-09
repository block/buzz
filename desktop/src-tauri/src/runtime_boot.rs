/// Installs the enlarged Tokio runtime required by MeshLLM before Tauri first
/// touches its async runtime. Non-mesh builds intentionally do nothing.
pub(crate) fn install_async_runtime() {
    #[cfg(feature = "mesh-llm")]
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::mesh_llm::MESH_WORKER_STACK_SIZE)
        .build()
    {
        Ok(runtime) => {
            tauri::async_runtime::set(runtime.handle().clone());
            // Keep the runtime alive for the process lifetime; dropping it
            // would shut down the workers Tauri now depends on.
            std::mem::forget(runtime);
            eprintln!(
                "buzz-mesh: installed tokio runtime with {} MiB worker stacks",
                crate::mesh_llm::MESH_WORKER_STACK_SIZE / (1024 * 1024)
            );
        }
        Err(error) => {
            // Tauri remains usable with its default runtime; only deeply nested
            // MeshLLM futures remain at risk of exhausting the smaller stack.
            eprintln!("buzz-mesh: failed to build big-stack tokio runtime, using default: {error}");
        }
    }
}
