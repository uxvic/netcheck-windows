//! Data usage from the OS's own accounting.
//!
//! Windows keeps a persistent, system-tracked usage ledger per network — accumulated
//! even while NetCheck isn't running — exposed via WinRT
//! `ConnectionProfile.GetNetworkUsageAsync`. This is per-network (not per-app), and the
//! most recent bucket is an estimate that settles. macOS has no equivalent public API,
//! so there the figure stays interface-counter based (handled by the Mac app, not here).

#[derive(serde::Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DataUsage {
    pub down_bytes: u64,
    pub up_bytes: u64,
    pub total_bytes: u64,
}

#[cfg(target_os = "windows")]
pub fn data_usage(period: &str) -> Result<DataUsage, String> {
    use chrono::{Datelike, Local, TimeZone};
    use windows::Foundation::DateTime;
    use windows::Networking::Connectivity::{
        DataUsageGranularity, NetworkInformation, NetworkUsageStates, TriStates,
    };
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    // WinRT calls need the calling thread initialised; ignore "already initialised".
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }

    // FILETIME / DateTime.UniversalTime are 100-ns ticks since 1601-01-01 UTC.
    fn to_universal_time(secs: i64, subsec_nanos: u32) -> i64 {
        (secs + 11_644_473_600) * 10_000_000 + (subsec_nanos as i64) / 100
    }

    let now = Local::now();
    let start = match period {
        "month" => Local
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single(),
        _ => now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|nd| Local.from_local_datetime(&nd).single()),
    }
    .ok_or_else(|| "could not compute period start".to_string())?;

    let start_dt = DateTime {
        UniversalTime: to_universal_time(start.timestamp(), start.timestamp_subsec_nanos()),
    };
    let end_dt = DateTime {
        UniversalTime: to_universal_time(now.timestamp(), now.timestamp_subsec_nanos()),
    };

    let profile = NetworkInformation::GetInternetConnectionProfile().map_err(|e| e.to_string())?;
    // DoNotCare on both axes = count roaming + shared, i.e. everything.
    let states = NetworkUsageStates {
        Roaming: TriStates::DoNotCare,
        Shared: TriStates::DoNotCare,
    };
    let usages = profile
        .GetNetworkUsageAsync(start_dt, end_dt, DataUsageGranularity::Total, states)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;

    let (mut down, mut up) = (0u64, 0u64);
    for u in usages {
        down += u.BytesReceived().unwrap_or(0);
        up += u.BytesSent().unwrap_or(0);
    }
    Ok(DataUsage {
        down_bytes: down,
        up_bytes: up,
        total_bytes: down + up,
    })
}

#[cfg(not(target_os = "windows"))]
pub fn data_usage(_period: &str) -> Result<DataUsage, String> {
    Err("OS data usage is only available on Windows".into())
}
