use actix_web::middleware::Logger;
use actix_web::{http::header, middleware::DefaultHeaders};

#[allow(dead_code)]
pub fn configure_middleware() -> Logger {
    Logger::default()
}

pub fn cors_headers() -> DefaultHeaders {
    DefaultHeaders::new()
        .add((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
        .add((
            header::ACCESS_CONTROL_ALLOW_METHODS,
            "GET, POST, PUT, DELETE, OPTIONS",
        ))
        .add((
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Content-Type, Authorization",
        ))
}
