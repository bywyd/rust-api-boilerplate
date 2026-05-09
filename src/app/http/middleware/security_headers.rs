use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::header::{HeaderName, HeaderValue},
    Error,
};
use futures::future::{ok, LocalBoxFuture, Ready};

/// Middleware that injects common security-related HTTP response headers on
/// every response, regardless of route or status code.
///
/// Headers added:
/// - `x-content-type-options: nosniff`
/// - `x-frame-options: DENY`
/// - `x-xss-protection: 1; mode=block`
/// - `referrer-policy: strict-origin-when-cross-origin`
/// - `content-security-policy: default-src 'self'`
/// - `permissions-policy: geolocation=(), microphone=(), camera=()`
pub struct SecurityHeaders;

impl<S, B> Transform<S, ServiceRequest> for SecurityHeaders
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = SecurityHeadersMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(SecurityHeadersMiddleware { service })
    }
}

pub struct SecurityHeadersMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for SecurityHeadersMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let fut = self.service.call(req);
        Box::pin(async move {
            let mut res = fut.await?;
            let headers = res.headers_mut();

            headers.insert(
                HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            );
            headers.insert(
                HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            );
            headers.insert(
                HeaderName::from_static("x-xss-protection"),
                HeaderValue::from_static("1; mode=block"),
            );
            headers.insert(
                HeaderName::from_static("referrer-policy"),
                HeaderValue::from_static("strict-origin-when-cross-origin"),
            );
            headers.insert(
                HeaderName::from_static("content-security-policy"),
                HeaderValue::from_static("default-src 'self'"),
            );
            headers.insert(
                HeaderName::from_static("permissions-policy"),
                HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
            );

            Ok(res)
        })
    }
}
