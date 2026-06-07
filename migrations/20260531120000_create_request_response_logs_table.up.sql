CREATE TABLE http_observability_logs (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    request_id VARCHAR(36) NOT NULL,
    method VARCHAR(10) NOT NULL,
    path VARCHAR(2048) NOT NULL,
    query_string TEXT NULL,
    ip_address VARCHAR(45) NULL,
    request_headers TEXT NULL,
    request_body LONGTEXT NULL,
    response_status INT NOT NULL,
    response_headers TEXT NULL,
    response_body LONGTEXT NULL,
    duration_ms INT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    KEY idx_request_id (request_id),
    KEY idx_method (method),
    KEY idx_created_at (created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
