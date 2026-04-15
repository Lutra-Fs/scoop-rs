use std::time::Duration;

use anyhow::Context;
use reqwest::{
    Client,
    blocking::Client as BlockingClient,
    header::{ACCEPT, ACCEPT_ENCODING, HeaderMap, HeaderValue, USER_AGENT},
};

pub fn build_http_client() -> anyhow::Result<Client> {
    Client::builder()
        .default_headers(default_headers())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .pool_max_idle_per_host(8)
        .tcp_nodelay(true)
        .build()
        .context("failed to build HTTP client")
}

pub fn build_blocking_http_client() -> anyhow::Result<BlockingClient> {
    BlockingClient::builder()
        .default_headers(default_headers())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .tcp_nodelay(true)
        .build()
        .context("failed to build blocking HTTP client")
}

fn default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, br"));
    headers.insert(USER_AGENT, HeaderValue::from_static("scoop-rs/0.1.0"));
    headers
}
