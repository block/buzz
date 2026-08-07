use url::Url;

/// Failure to parse or validate a Buzz community endpoint.
#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    /// The input is not an absolute URL.
    #[error("invalid community URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    /// Only HTTP and WebSocket relay URL schemes are supported.
    #[error("unsupported community URL scheme `{0}`; use http, https, ws, or wss")]
    UnsupportedScheme(String),

    /// The URL has no host authority.
    #[error("community URL must include a host authority")]
    MissingHost,

    /// Credentials in community or resource URLs could be disclosed accidentally.
    #[error("community URLs must not contain embedded credentials")]
    EmbeddedCredentials,

    /// A community base identifies an authority, not a nested path.
    #[error("community URL must not contain a path, query, or fragment")]
    NonRootBase,

    /// A resource resolves outside the client's bound community authority.
    #[error("resource authority `{actual}` does not match community authority `{expected}`")]
    CrossAuthority {
        /// Normalized authority expected by the client.
        expected: String,
        /// Normalized authority supplied by the resource.
        actual: String,
    },
}

/// Normalized endpoints for one Buzz community authority.
///
/// HTTP input is paired with WebSocket input (`http`/`ws` or `https`/`wss`),
/// default ports are normalized, and all derived operation URLs retain the
/// same host, effective port, and transport-security class.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommunityEndpoint {
    http_base: Url,
    websocket: Url,
}

impl CommunityEndpoint {
    /// Parse and normalize an HTTP or WebSocket community URL.
    ///
    /// A base URL must identify only an authority: embedded credentials,
    /// non-root paths, query strings, and fragments are rejected.
    pub fn parse(input: &str) -> Result<Self, EndpointError> {
        let mut input = Url::parse(input.trim())?;
        validate_supported_scheme(input.scheme())?;
        validate_credentials(&input)?;
        if input.host_str().is_none() {
            return Err(EndpointError::MissingHost);
        }
        if !matches!(input.path(), "" | "/")
            || input.query().is_some()
            || input.fragment().is_some()
        {
            return Err(EndpointError::NonRootBase);
        }

        let secure = matches!(input.scheme(), "https" | "wss");
        input
            .set_scheme(if secure { "https" } else { "http" })
            .map_err(|_| EndpointError::UnsupportedScheme(input.scheme().to_string()))?;
        input.set_path("/");
        let mut websocket = input.clone();
        websocket
            .set_scheme(if secure { "wss" } else { "ws" })
            .map_err(|_| EndpointError::UnsupportedScheme(input.scheme().to_string()))?;

        Ok(Self {
            http_base: input,
            websocket,
        })
    }

    /// Return the normalized HTTP base URL.
    pub fn http_base_url(&self) -> &Url {
        &self.http_base
    }

    /// Return the normalized WebSocket base URL.
    pub fn websocket_url(&self) -> &Url {
        &self.websocket
    }

    /// Derive the authenticated query endpoint.
    pub fn query_url(&self) -> Url {
        self.operation_url("query")
    }

    /// Derive the stored-event submission endpoint.
    pub fn events_url(&self) -> Url {
        self.operation_url("events")
    }

    /// Derive the media upload endpoint.
    pub fn upload_url(&self) -> Url {
        self.operation_url("upload")
    }

    /// Parse a resource URL and require it to retain this community authority.
    ///
    /// Both HTTP and WebSocket forms of the configured transport-security
    /// class are accepted. A secure client never accepts an insecure resource,
    /// even when the host and explicit port happen to match.
    pub fn same_authority_url(&self, input: &str) -> Result<Url, EndpointError> {
        let resource = Url::parse(input.trim())?;
        validate_supported_scheme(resource.scheme())?;
        validate_credentials(&resource)?;
        if resource.host_str().is_none() {
            return Err(EndpointError::MissingHost);
        }

        let expected = authority_key(&self.http_base);
        let actual = authority_key(&resource);
        if expected != actual {
            return Err(EndpointError::CrossAuthority {
                expected: authority_label(&self.http_base),
                actual: authority_label(&resource),
            });
        }
        Ok(resource)
    }

    fn operation_url(&self, path: &str) -> Url {
        let mut url = self.http_base.clone();
        url.set_path(path);
        url
    }
}

fn validate_supported_scheme(scheme: &str) -> Result<(), EndpointError> {
    match scheme {
        "http" | "https" | "ws" | "wss" => Ok(()),
        other => Err(EndpointError::UnsupportedScheme(other.to_string())),
    }
}

fn validate_credentials(url: &Url) -> Result<(), EndpointError> {
    if !url.username().is_empty() || url.password().is_some() {
        Err(EndpointError::EmbeddedCredentials)
    } else {
        Ok(())
    }
}

fn authority_key(url: &Url) -> (bool, &str, Option<u16>) {
    (
        matches!(url.scheme(), "https" | "wss"),
        url.host_str().unwrap_or_default(),
        effective_port(url),
    )
}

fn effective_port(url: &Url) -> Option<u16> {
    url.port().or_else(|| match url.scheme() {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    })
}

fn authority_label(url: &Url) -> String {
    let security = if matches!(url.scheme(), "https" | "wss") {
        "secure"
    } else {
        "insecure"
    };
    format!(
        "{security}://{}:{}",
        url.host_str().unwrap_or("<missing>"),
        effective_port(url)
            .map(|port| port.to_string())
            .unwrap_or_else(|| "<unknown>".into())
    )
}
