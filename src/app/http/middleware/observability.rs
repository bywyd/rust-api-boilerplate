use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::header::HeaderValue,
    web, Error, HttpMessage,
};
use futures::future::{ok, LocalBoxFuture, Ready};
use futures::StreamExt;
use std::rc::Rc;
use std::time::Instant;
use uuid::Uuid;
use crate::app::state::AppState;
use actix_web::body::{to_bytes, MessageBody, BoxBody};

/// Middleware that logs incoming requests and outgoing responses (including bodies)
/// to the `http_observability_logs` database table.
pub struct ObservabilityLogger;

impl<S, B> Transform<S, ServiceRequest> for ObservabilityLogger
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = ObservabilityLoggerMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(ObservabilityLoggerMiddleware {
            service: Rc::new(service),
        })
    }
}

pub struct ObservabilityLoggerMiddleware<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for ObservabilityLoggerMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        let start_time = Instant::now();
        let request_id = Uuid::new_v4().to_string();

        let method = req.method().to_string();
        let path = req.path().to_string();

        if path.starts_with("/api/observability") {
            return Box::pin(async move {
                let res = service.call(req).await?;
                Ok(res.map_into_boxed_body())
            });
        }


        let query_string = if req.query_string().is_empty() {
            None
        } else {
            Some(req.query_string().to_string())
        };
        let ip_address = req
            .connection_info()
            .realip_remote_addr()
            .map(|ip| ip.to_string());

        let mut req_headers_map = serde_json::Map::new();
        for (name, val) in req.headers() {
            if let Ok(val_str) = val.to_str() {
                req_headers_map.insert(name.to_string(), serde_json::Value::String(val_str.to_string()));
            }
        }
        let req_headers_str = serde_json::to_string(&req_headers_map).unwrap_or_default();

        let state = req.app_data::<web::Data<AppState>>().cloned();

        if let Some(state) = state {
            if !state.config.logging.http_request_logging.enabled {
                return Box::pin(async move {
                    let res = service.call(req).await?;
                    Ok(res.map_into_boxed_body())
                });
            }
        }

        let content_type = req
            .headers()
            .get(actix_web::http::header::CONTENT_TYPE)
            .and_then(|val| val.to_str().ok())
            .unwrap_or("")
            .to_string();

        Box::pin(async move {
            let mut req_body_str = None;

            // Log body only if it's text/json and small
            if is_text_or_json(&content_type) {
                let mut payload = req.take_payload();
                let mut body_bytes = actix_web::web::BytesMut::new();
                while let Some(chunk) = payload.next().await {
                    match chunk {
                        Ok(bytes) => {
                            if body_bytes.len() + bytes.len() <= 65536 {
                                body_bytes.extend_from_slice(&bytes);
                            } else {
                                req_body_str = Some("[Request body too large, truncated]".to_string());
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                
                let body = body_bytes.freeze();
                if req_body_str.is_none() {
                    req_body_str = Some(String::from_utf8_lossy(&body).into_owned());
                }

                // Restore payload for downstream handlers
                let body_stream = futures::stream::once(async move { Ok::<_, actix_web::error::PayloadError>(body) });
                let stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<actix_web::web::Bytes, actix_web::error::PayloadError>> + 'static>> = Box::pin(body_stream);
                let payload = actix_web::dev::Payload::Stream { payload: stream };
                req.set_payload(payload);
            } else if !content_type.is_empty() {
                req_body_str = Some(format!("[Binary/Multipart data (content-type: {})]", content_type));
            }

            // Call the inner service
            let res = service.call(req).await;

            match res {
                Ok(response) => {
                    let duration_ms = start_time.elapsed().as_millis() as i64;
                    let (req, res) = response.into_parts();
                    
                    let status = res.status().as_u16() as i32;

                    let mut res_headers_map = serde_json::Map::new();
                    for (name, val) in res.headers() {
                        if let Ok(val_str) = val.to_str() {
                            res_headers_map.insert(name.to_string(), serde_json::Value::String(val_str.to_string()));
                        }
                    }
                    let res_headers_str = serde_json::to_string(&res_headers_map).unwrap_or_default();

                    let res_content_type = res
                        .headers()
                        .get(actix_web::http::header::CONTENT_TYPE)
                        .and_then(|val| val.to_str().ok())
                        .unwrap_or("")
                        .to_string();

                    let res_status = res.status();
                    let res_headers = res.headers().clone();
                    let res_body = res.into_body();
                    
                    let mut res_body_str = None;
                    let boxed_body = if is_text_or_json(&res_content_type) {
                        match to_bytes(res_body).await {
                            Ok(bytes) => {
                                if bytes.len() <= 65536 {
                                    res_body_str = Some(String::from_utf8_lossy(&bytes).into_owned());
                                } else {
                                    res_body_str = Some("[Response body too large, truncated]".to_string());
                                }
                                BoxBody::new(bytes)
                            }
                            Err(_) => BoxBody::new(actix_web::web::Bytes::new()),
                        }
                    } else {
                        if !res_content_type.is_empty() {
                            res_body_str = Some(format!("[Binary/Multipart data (content-type: {})]", res_content_type));
                        }
                        res_body.boxed()
                    };

                    if let Some(state) = state {
                        let db = state.db.clone();
                        let request_id_clone = request_id.clone();
                        let method_clone = method.clone();
                        let path_clone = path.clone();
                        let query_string_clone = query_string.clone();
                        let ip_address_clone = ip_address.clone();
                        let req_headers_str_clone = req_headers_str.clone();
                        let req_body_str_clone = req_body_str.clone();
                        let res_headers_str_clone = res_headers_str.clone();
                        let res_body_str_clone = res_body_str.clone();

                        tokio::spawn(async move {
                            let res_insert = sqlx::query(
                                "INSERT INTO http_observability_logs (request_id, method, path, query_string, ip_address, request_headers, request_body, response_status, response_headers, response_body, duration_ms) \
                                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
                            )
                            .bind(request_id_clone)
                            .bind(method_clone)
                            .bind(path_clone)
                            .bind(query_string_clone)
                            .bind(ip_address_clone)
                            .bind(req_headers_str_clone)
                            .bind(req_body_str_clone)
                            .bind(status)
                            .bind(res_headers_str_clone)
                            .bind(res_body_str_clone)
                            .bind(duration_ms)
                            .execute(&db)
                            .await;

                            if let Err(e) = res_insert {
                                tracing::error!("Failed to save HTTP request observability log: {:?}", e);
                            }
                        });
                    }

                    let mut actix_res = actix_web::HttpResponse::new(res_status);
                    *actix_res.headers_mut() = res_headers;
                    let actix_res = actix_res.set_body(boxed_body);

                    let mut new_res = ServiceResponse::new(req, actix_res);
                    if let Ok(hdr) = HeaderValue::from_str(&request_id) {
                        new_res.headers_mut().insert(
                            actix_web::http::header::HeaderName::from_static("x-request-id"),
                            hdr,
                        );
                    }
                    Ok(new_res)
                }
                Err(e) => {
                    let duration_ms = start_time.elapsed().as_millis() as i64;
                    let error_msg = format!("{:?}", e);

                    if let Some(state) = state {
                        let db = state.db.clone();
                        let request_id_clone = request_id.clone();
                        let method_clone = method.clone();
                        let path_clone = path.clone();
                        let query_string_clone = query_string.clone();
                        let ip_address_clone = ip_address.clone();
                        let req_headers_str_clone = req_headers_str.clone();
                        let req_body_str_clone = req_body_str.clone();

                        tokio::spawn(async move {
                            let res_insert = sqlx::query(
                                "INSERT INTO http_observability_logs (request_id, method, path, query_string, ip_address, request_headers, request_body, response_status, response_headers, response_body, duration_ms) \
                                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
                            )
                            .bind(request_id_clone)
                            .bind(method_clone)
                            .bind(path_clone)
                            .bind(query_string_clone)
                            .bind(ip_address_clone)
                            .bind(req_headers_str_clone)
                            .bind(req_body_str_clone)
                            .bind(500)
                            .bind("{}")
                            .bind(Some(format!("[Internal Error]: {}", error_msg)))
                            .bind(duration_ms)
                            .execute(&db)
                            .await;

                            if let Err(err_db) = res_insert {
                                tracing::error!("Failed to save HTTP request observability log (on error): {:?}", err_db);
                            }
                        });
                    }

                    Err(e)
                }
            }
        })
    }
}

fn is_text_or_json(content_type: &str) -> bool {
    let ct = content_type.to_lowercase();
    ct.contains("json") || ct.contains("text") || ct.contains("xml") || ct.contains("urlencoded")
}
