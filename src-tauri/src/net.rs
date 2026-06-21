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

#[derive(PartialEq)]
enum Probe {
    Online,
    Portal,
    Fail,
}

/// Reachability via several independent endpoints raced together, so one provider's
/// hiccup never reads as "offline" on a working link:
/// - any endpoint returns its genuine success payload -> Online
/// - else any endpoint is intercepted (redirect / unexpected body) -> CaptivePortal
/// - all fail but a raw TCP connect to a public IP works -> CaptivePortal (DNS/proxy broken)
/// - everything fails -> NoInternet
pub async fn check_reachability(client: &reqwest::Client) -> Reachability {
    let (cf, g204, apple) = tokio::join!(
        probe_body(client, "https://cloudflare.com/cdn-cgi/trace", &["h=", "ip="]),
        probe_204(client, "http://www.gstatic.com/generate_204"),
        probe_body(client, "http://captive.apple.com/hotspot-detect.html", &["Success"]),
    );
    let results = [cf, g204, apple];
    if results.iter().any(|r| *r == Probe::Online) {
        return Reachability::Online;
    }
    if results.iter().any(|r| *r == Probe::Portal) {
        return Reachability::CaptivePortal;
    }
    // All HTTP probes failed: DNS/proxy broken (TCP-by-IP still works) vs. full blackhole.
    if tcp_probe("1.1.1.1:443").await.is_ok() || tcp_probe("8.8.8.8:443").await.is_ok() {
        Reachability::CaptivePortal
    } else {
        Reachability::NoInternet
    }
}

/// Endpoint that returns a known plaintext body when reached directly (Cloudflare
/// trace, Apple hotspot-detect). A redirect or a body missing the markers = intercepted.
async fn probe_body(client: &reqwest::Client, url: &str, needles: &[&str]) -> Probe {
    match timeout(PROBE_TIMEOUT, client.get(url).send()).await {
        Ok(Ok(resp)) => {
            if resp.status().is_redirection() {
                return Probe::Portal;
            }
            match resp.text().await {
                Ok(b) if needles.iter().any(|n| b.contains(n)) => Probe::Online,
                Ok(_) => Probe::Portal,
                Err(_) => Probe::Fail,
            }
        }
        _ => Probe::Fail,
    }
}

/// A `generate_204` endpoint: exactly 204 when reached directly; a 200-with-login-page
/// or a redirect means a captive portal intercepted it.
async fn probe_204(client: &reqwest::Client, url: &str) -> Probe {
    match timeout(PROBE_TIMEOUT, client.get(url).send()).await {
        Ok(Ok(resp)) => {
            let s = resp.status();
            if s == reqwest::StatusCode::NO_CONTENT {
                Probe::Online
            } else if s.is_redirection() || s.is_success() {
                Probe::Portal
            } else {
                Probe::Fail
            }
        }
        _ => Probe::Fail,
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
    for (name, data) in nets.iter() {
        // Count only the physical uplink(s). A VPN's real bytes already traverse the
        // physical NIC, so adding the tunnel adapter too would double-count; loopback and
        // virtual switches add idle noise. Both inflate the rate that drives the globe.
        if !is_physical_uplink(name) {
            continue;
        }
        rx += data.received();
        tx += data.transmitted();
    }
    let secs = interval.as_secs_f64();
    Throughput {
        down_mbps: (rx as f64 * 8.0) / (secs * 1_000_000.0),
        up_mbps: (tx as f64 * 8.0) / (secs * 1_000_000.0),
    }
}

/// Heuristic: exclude loopback, virtual switches, container bridges, and VPN tunnel
/// adapters by name, leaving the physical Ethernet/Wi-Fi uplink(s).
fn is_physical_uplink(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const VIRTUAL: &[&str] = &[
        "loopback", "pseudo-interface", "isatap", "teredo", "6to4", "vethernet",
        "vmware", "virtualbox", "vbox", "hyper-v", "wsl", "docker", "tailscale",
        "wireguard", "zerotier", "openvpn", "ppp adapter", "tunnel",
    ];
    !VIRTUAL.iter().any(|v| n.contains(v))
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
