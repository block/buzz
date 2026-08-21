use axum::{
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// The `WWW-Authenticate` header value for 401 responses. Defaults to
    /// `Bearer`; set to `Nostr` in NIP-98 mode via `with_www_authenticate`.
    pub www_authenticate: &'static str,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: &'static str,
    message: String,
    request_id: uuid::Uuid,
}

impl ApiError {
    pub fn bad_request(code: &'static str, message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.to_owned(),
            www_authenticate: "Bearer",
        }
    }

    pub fn conflict(message: &str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict",
            message: message.to_owned(),
            www_authenticate: "Bearer",
        }
    }

    pub fn unprocessable(message: &str) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "enforcement_failed",
            message: message.to_owned(),
            www_authenticate: "Bearer",
        }
    }

    pub fn forbidden() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "request is not authorized".to_owned(),
            www_authenticate: "Bearer",
        }
    }

    pub fn forbidden_with_message(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: message.to_owned(),
            www_authenticate: "Bearer",
        }
    }

    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "a valid admin credential is required".to_owned(),
            www_authenticate: "Bearer",
        }
    }

    pub fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "record was not found".to_owned(),
            www_authenticate: "Bearer",
        }
    }

    pub fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "request failed".to_owned(),
            www_authenticate: "Bearer",
        }
    }

    /// Override the `WWW-Authenticate` challenge value. Used by NIP-98 mode
    /// to advertise `Nostr` instead of `Bearer`.
    pub fn with_www_authenticate(mut self, challenge: &'static str) -> Self {
        self.www_authenticate = challenge;
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                    request_id: uuid::Uuid::new_v4(),
                },
            }),
        )
            .into_response();
        // RFC 9110 requires a challenge on every 401 so clients know which
        // scheme to present.
        if self.status == StatusCode::UNAUTHORIZED {
            if let Ok(value) = HeaderValue::from_str(self.www_authenticate) {
                response
                    .headers_mut()
                    .insert(axum::http::header::WWW_AUTHENTICATE, value);
            }
        }
        response
    }
}

impl From<buzz_db::DbError> for ApiError {
    fn from(_: buzz_db::DbError) -> Self {
        Self::internal()
    }
}
