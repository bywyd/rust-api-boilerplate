#![allow(dead_code)]

use crate::infra::config::app_config::HttpClientConfig;
use anyhow::Result;
use reqwest::Client;
use serde::{Serialize, de::DeserializeOwned};
use std::time::Duration;

/// Thin wrapper around `reqwest::Client` providing typed request helpers.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HttpClient {
    inner: Client,
}

impl HttpClient {
    #[allow(dead_code)]
    pub fn new(cfg: &HttpClientConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_seconds))
            .connect_timeout(Duration::from_secs(cfg.connect_timeout_seconds))
            .build()?;

        Ok(Self { inner: client })
    }

    /// Perform a GET request and deserialize the JSON response body.
    pub async fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let response = self
            .inner
            .get(url)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json::<T>().await?)
    }

    /// Perform a POST request with a JSON body and deserialize the JSON response.
    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        let response = self
            .inner
            .post(url)
            .json(body)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json::<T>().await?)
    }

    /// Perform a PUT request with a JSON body and deserialize the JSON response.
    pub async fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        let response = self
            .inner
            .put(url)
            .json(body)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json::<T>().await?)
    }

    /// Perform a DELETE request.
    pub async fn delete(&self, url: &str) -> Result<()> {
        self.inner
            .delete(url)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}
