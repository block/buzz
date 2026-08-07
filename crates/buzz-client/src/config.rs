use std::time::Duration;

use crate::ClientError;

/// Bounded retry settings for authenticated Buzz requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub(crate) max_attempts: u32,
    pub(crate) base_delay: Duration,
    pub(crate) max_delay: Duration,
}

impl RetryPolicy {
    /// Construct a bounded retry policy.
    pub fn new(
        max_attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
    ) -> Result<Self, ClientError> {
        if !(1..=10).contains(&max_attempts) {
            return Err(ClientError::Configuration(
                "retry max_attempts must be between 1 and 10".into(),
            ));
        }
        if base_delay.is_zero() {
            return Err(ClientError::Configuration(
                "retry base_delay must be greater than zero".into(),
            ));
        }
        if max_delay < base_delay {
            return Err(ClientError::Configuration(
                "retry max_delay must be at least base_delay".into(),
            ));
        }
        if max_delay > Duration::from_secs(300) {
            return Err(ClientError::Configuration(
                "retry max_delay must not exceed 300 seconds".into(),
            ));
        }
        Ok(Self {
            max_attempts,
            base_delay,
            max_delay,
        })
    }

    /// Maximum total attempts, including the initial attempt.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Initial backoff delay before applying exponential growth.
    pub fn base_delay(&self) -> Duration {
        self.base_delay
    }

    /// Maximum delay between attempts.
    pub fn max_delay(&self) -> Duration {
        self.max_delay
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(10),
        }
    }
}

/// Explicit HTTP and retry configuration for a client session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientConfig {
    pub(crate) request_timeout: Duration,
    pub(crate) connect_timeout: Duration,
    pub(crate) retry: RetryPolicy,
}

impl ClientConfig {
    /// Construct validated client configuration.
    pub fn new(
        request_timeout: Duration,
        connect_timeout: Duration,
        retry: RetryPolicy,
    ) -> Result<Self, ClientError> {
        if request_timeout.is_zero() {
            return Err(ClientError::Configuration(
                "request_timeout must be greater than zero".into(),
            ));
        }
        if connect_timeout.is_zero() {
            return Err(ClientError::Configuration(
                "connect_timeout must be greater than zero".into(),
            ));
        }
        if connect_timeout > request_timeout {
            return Err(ClientError::Configuration(
                "connect_timeout must not exceed request_timeout".into(),
            ));
        }
        Ok(Self {
            request_timeout,
            connect_timeout,
            retry,
        })
    }

    /// Overall HTTP request timeout.
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// HTTP connection-establishment timeout.
    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// Bounded retry policy.
    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(15),
            retry: RetryPolicy::default(),
        }
    }
}
