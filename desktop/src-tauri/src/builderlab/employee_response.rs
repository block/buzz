//! Assertion reflection and lifetime boundary shared by JSON and streaming media.
use super::*;
use futures_util::{StreamExt, TryStreamExt};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct ProtectedAssertion(
    pub(crate) String,
    pub(crate) CancellationToken,
    pub(crate) i64,
);

pub(crate) async fn employee_response_end(scope: Option<ProtectedAssertion>) {
    if let Some(scope) = scope {
        let lifetime = Duration::from_secs(
            scope
                .2
                .saturating_sub(chrono::Utc::now().timestamp())
                .max(0) as u64,
        );
        tokio::select! { _ = scope.1.cancelled() => {}, _ = tokio::time::sleep(lifetime) => {} }
    } else {
        std::future::pending::<()>().await;
    }
}

pub(crate) async fn guard_employee_response<T>(
    scope: Option<ProtectedAssertion>,
    future: impl std::future::Future<Output = T>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = employee_response_end(scope) => Err("Employee relay authorization changed".to_owned()),
        result = future => Ok(result),
    }
}

pub(super) fn filter_response(response: reqwest::Response) -> Result<reqwest::Response, String> {
    use reqwest::ResponseBuilderExt;
    let scope = response
        .extensions()
        .get::<ProtectedAssertion>()
        .cloned()
        .ok_or("Missing employee response scope")?;
    if response.headers().iter().any(|(_, value)| {
        value
            .as_bytes()
            .windows(scope.0.len())
            .any(|part| part == scope.0.as_bytes())
    }) {
        return Err("Invalid employee relay response".to_owned());
    }
    let mut builder = tauri::http::Response::builder()
        .status(response.status())
        .version(response.version())
        .url(response.url().clone());
    *builder.headers_mut().ok_or("Invalid relay headers")? = response.headers().clone();
    builder
        .extensions_mut()
        .ok_or("Invalid relay metadata")?
        .extend(response.extensions().clone());
    let stream = filter_stream(
        response.bytes_stream().map_err(std::io::Error::other),
        scope,
    );
    let body = reqwest::Body::wrap_stream(stream);
    let response = builder
        .body(body)
        .map_err(|_| "Invalid employee relay response")?;
    Ok(response.into())
}

fn filter_stream<S>(
    stream: S,
    scope: ProtectedAssertion,
) -> impl futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + Unpin,
{
    // Retain one possible token prefix across chunk boundaries. Never release
    // a complete reflected credential, even when the relay splits its bytes.
    futures_util::stream::try_unfold(
        (stream, Vec::<u8>::new(), scope, false),
        |(mut stream, mut pending, scope, mut done)| async move {
            loop {
                if done {
                    return if pending.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some((
                            bytes::Bytes::from(pending),
                            (stream, Vec::new(), scope, true),
                        )))
                    };
                }
                match guard_employee_response(Some(scope.clone()), stream.next())
                    .await
                    .map_err(std::io::Error::other)?
                {
                    Some(chunk) => {
                        pending.extend_from_slice(&chunk?);
                        if pending
                            .windows(scope.0.len())
                            .any(|part| part == scope.0.as_bytes())
                        {
                            return Err(std::io::Error::other("Invalid employee relay response"));
                        }
                        let available = pending
                            .len()
                            .saturating_sub(scope.0.len().saturating_sub(1));
                        if available > 0 {
                            let remaining = pending.split_off(available);
                            return Ok(Some((
                                bytes::Bytes::from(pending),
                                (stream, remaining, scope, false),
                            )));
                        }
                    }
                    None => done = true,
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn split_reflection_is_never_reassembled_outside_native() {
        let scope = ProtectedAssertion(
            "private.media.assertion".to_owned(),
            CancellationToken::new(),
            chrono::Utc::now().timestamp() + 60,
        );
        let stream = futures_util::stream::iter([
            Ok(bytes::Bytes::from_static(b"safe-prefix-private.med")),
            Ok(bytes::Bytes::from_static(b"ia.assertion-after")),
        ]);
        let stream = filter_stream(stream, scope);
        tokio::pin!(stream);
        let mut released = Vec::new();
        let mut failed = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => released.extend_from_slice(&chunk),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed);
        assert!(!released
            .windows(b"private.media.assertion".len())
            .any(|part| part == b"private.media.assertion"));
    }
    #[test]
    fn reflected_headers_are_rejected_before_either_media_proxy_can_forward_them() {
        for name in ["content-type", "etag", "content-range"] {
            let mut response: reqwest::Response = tauri::http::Response::builder()
                .header(name, "private.media.assertion")
                .body("safe")
                .unwrap()
                .into();
            response.extensions_mut().insert(ProtectedAssertion(
                "private.media.assertion".to_owned(),
                CancellationToken::new(),
                chrono::Utc::now().timestamp() + 60,
            ));
            assert!(filter_response(response).is_err());
        }
    }
    #[tokio::test]
    async fn ordinary_binary_chunks_are_preserved_exactly() {
        let scope = ProtectedAssertion(
            "private.media.assertion".to_owned(),
            CancellationToken::new(),
            chrono::Utc::now().timestamp() + 60,
        );
        let input = vec![7u8; 256 * 1024];
        let stream = futures_util::stream::iter(
            input
                .chunks(4096)
                .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk))),
        );
        let output = filter_stream(stream, scope)
            .try_fold(Vec::new(), |mut result, chunk| async move {
                result.extend_from_slice(&chunk);
                Ok(result)
            })
            .await
            .unwrap();
        assert_eq!(output, input);
    }
}
