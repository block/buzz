use super::ImageFetchError;

pub(super) async fn retry_transient_image_fetch<F, Fut>(
    mut fetch: F,
) -> Result<(String, String), ImageFetchError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(String, String), ImageFetchError>>,
{
    let first = fetch().await;
    if matches!(
        first,
        Err(ImageFetchError::Transient {
            retry_inline: true,
            ..
        })
    ) {
        return fetch().await;
    }
    first
}

#[cfg(test)]
mod tests {
    use super::retry_transient_image_fetch;
    use crate::commands::link_preview::ImageFetchError;
    use std::{cell::Cell, time::Duration};

    #[tokio::test]
    async fn retries_one_transient_failure_inline() {
        let attempts = Cell::new(0);
        let result = retry_transient_image_fetch(|| {
            let attempt = attempts.get() + 1;
            attempts.set(attempt);
            async move {
                if attempt == 1 {
                    Err(ImageFetchError::Transient {
                        retry_after: None,
                        retry_inline: true,
                    })
                } else {
                    Ok(("image".to_string(), "example.com".to_string()))
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(attempts.get(), 2);
    }

    #[tokio::test]
    async fn does_not_retry_rate_limits_inline() {
        let attempts = Cell::new(0);
        let result = retry_transient_image_fetch(|| {
            attempts.set(attempts.get() + 1);
            async {
                Err(ImageFetchError::Transient {
                    retry_after: Some(Duration::from_secs(60)),
                    retry_inline: false,
                })
            }
        })
        .await;

        assert_eq!(
            result,
            Err(ImageFetchError::Transient {
                retry_after: Some(Duration::from_secs(60)),
                retry_inline: false,
            })
        );
        assert_eq!(attempts.get(), 1);
    }
}
