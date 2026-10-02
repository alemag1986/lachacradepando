use axum::{http::StatusCode, response::{IntoResponse, Response}};

#[derive(Debug)]
pub struct AppError(pub anyhow::Error);

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self { Self(error) }
}
impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self { Self(error.into()) }
}
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("request failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "Request could not be completed").into_response()
    }
}
