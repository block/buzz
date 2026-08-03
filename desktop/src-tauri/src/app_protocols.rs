use tauri::{Builder, Wry};

use crate::{commands::*, media_proxy};

pub fn register(builder: Builder<Wry>) -> Builder<Wry> {
    let builder = builder.register_asynchronous_uri_scheme_protocol(
        "buzz-media",
        |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let response = media_proxy::handle_buzz_media(&app, &request).await;
                responder.respond(response);
            });
        },
    );
    #[cfg(not(target_os = "windows"))]
    let builder = builder.register_asynchronous_uri_scheme_protocol(
        "buzz-mcp-app",
        |ctx, request, responder| {
            responder.respond(handle_mcp_app_protocol(ctx.app_handle(), &request));
        },
    );

    builder.manage(McpAppHostState::default())
}
