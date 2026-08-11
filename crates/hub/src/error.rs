use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use pinglake_protocol::ApiError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{1}")]
    Public(StatusCode, String),
    #[error("internal server error")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::Public(StatusCode::BAD_REQUEST, message.into())
    }

    pub fn unauthorized() -> Self {
        Self::Public(StatusCode::UNAUTHORIZED, "unauthorized".to_owned())
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::Public(StatusCode::TOO_MANY_REQUESTS, message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::Public(StatusCode::NOT_FOUND, message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Public(StatusCode::CONFLICT, message.into())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Internal(error.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::Public(status, message) => {
                (status, Json(ApiError { error: message })).into_response()
            }
            Self::Internal(error) => {
                tracing::error!(error = %error, "request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiError {
                        error: "internal server error".to_owned(),
                    }),
                )
                    .into_response()
            }
        }
    }
}
