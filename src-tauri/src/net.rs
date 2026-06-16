//! Networking core: reachability, latency, live throughput, and a one-shot speed test.
//!
//! The whole point of NetCheck is catching "link up but no real internet" (a dead
//! VPN, a captive portal). So reachability is an actual HTTPS probe, not link status.

use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::{timeout, Instant};

/// Result of a reachability probe — the three states that actually matter.
#[derive(Debug, Clone, PartialEq)]
pub enum Reachability {
    Online,
    CaptivePortal,
    NoInternet,
}

/// Live up/down rate in megabits per second (decimal Mbps).
pub struct Throughput {
    pub down_mbps: f64,
    pub up_mbps: f64,
}

/// Two reqwest clients: a strict short-timeout one for probes (no redirect-follow so
/// we can *see* a captive-portal 3xx), and a permissive long-timeout one for the
/// large speed-test download.
pub struct Clients {
    pub probe: reqwest::Client,
    pub speed: reqwest::Client,
}

impl Clients {
    pub fn new() -> Self {
        let probe = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("build probe client");
        let speed = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build speed client");
        Self { probe, speed }
    }
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// HTTPS probe to Cloudflare's trace endpoint, with a raw-TCP fallback.
/// - genuine trace body  -> Online
/// - 3xx redirect / unexpected body -> CaptivePortal (sign-in wall)
/// - probe errors but TCP-by-IP works -> CaptivePortal (DNS/proxy broken)
/// - everything fails -> NoInternet
pub async fn check_reachability(client: &reqwest::Client) -> Reachability {
    match timeout(
        PROBE_TIMEOUT,
        client.get("https://cloudflare.com/cdn-cgi/trace").send(),
    )
    .await
    {
        Ok(Ok(resp)) => {
            if resp.status().is_redirection() {
                return Reachability::CaptivePortal;
            }
            match resp.text().await {
                Ok(body) if body.contains("h=") || body.contains("ip=") => Reachability::Online,
                Ok(_) => Reachability::CaptivePortal,
                Err(_) => Reachability::NoInternet,
            }
        }
        Ok(Err(_)) | Err(_) => {
            if tcp_probe("1.1.1.1:443").await.is_ok() {
                Reachability::CaptivePortal
            } else {
                Reachability::NoInternet
            }
        }
    }
}

async fn tcp_probe(addr: &str) -> std::io::Result<()> {
    match timeout(PROBE_TIMEOUT, TcpStream::connect(addr)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "tcp connect timed out",
        )),
    }
}

/// TCP-connect round-trip to a raw IP, in milliseconds. No ICMP (raw sockets need
/// admin on Windows); connecting to an IP avoids folding DNS into the number.
pub async fn measure_latency_ms(addr: &str) -> Option<f64> {
    let start = Instant::now();
    match timeout(Duration::from_secs(2), TcpStream::connect(addr)).await {
        Ok(Ok(_)) => Some(start.elapsed().as_secs_f64() * 1000.0),
        _ => None,
    }
}

/// Sample OS interface byte counters over `interval` and return aggregate Mbps.
/// sysinfo 0.39: `received()`/`transmitted()` are deltas since the last refresh,
/// and `refresh` takes a `remove_not_listed_interfaces` bool.
pub async fn sample_throughput(interval: Duration) -> Throughput {
    use sysinfo::Networks;
    let mut nets = Networks::new_with_refreshed_list();
    tokio::time::sleep(interval).await;
    nets.refresh(true);

    let (mut rx, mut tx) = (0u64, 0u64);
    for (_name, data) in nets.iter() {
        rx += data.received();
        tx += data.transmitted();
    }
    let secs = interval.as_secs_f64();
    Throughput {
        down_mbps: (rx as f64 * 8.0) / (secs * 1_000_000.0),
        up_mbps: (tx as f64 * 8.0) / (secs * 1_000_000.0),
    }
}

/// One-shot download throughput test against Cloudflare. Uses `Response::chunk()`
/// (no `stream` feature / no futures-util needed) and times the body transfer.
pub async fn run_speed_test(client: &reqwest::Client, bytes: u64) -> Result<f64, String> {
    let url = format!("https://speed.cloudflare.com/__down?bytes={bytes}");
    let mut resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;

    let start = Instant::now();
    let mut total: u64 = 0;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        total += chunk.len() as u64;
    }
    let secs = start.elapsed().as_secs_f64().max(0.001);
    Ok((total as f64 * 8.0) / (secs * 1_000_000.0))
}
