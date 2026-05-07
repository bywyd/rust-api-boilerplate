CREATE TABLE IF NOT EXISTS jobs (
    id           CHAR(36)      NOT NULL,
    job_type     VARCHAR(255)  NOT NULL,
    payload      LONGTEXT      NOT NULL,
    status       VARCHAR(50)   NOT NULL DEFAULT 'pending',
    attempts     INT           NOT NULL DEFAULT 0,
    max_attempts INT           NOT NULL DEFAULT 3,
    run_at       DATETIME(3)   NOT NULL,
    created_at   DATETIME(3)   NOT NULL,
    failed_at    DATETIME(3)   NULL,
    error        TEXT          NULL,

    PRIMARY KEY (id),
    INDEX idx_jobs_status_run_at (status, run_at)
) ENGINE = InnoDB
  DEFAULT CHARSET = utf8mb4
  COLLATE = utf8mb4_unicode_ci;
