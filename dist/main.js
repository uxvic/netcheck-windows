// NetCheck flyout. Talks to the Rust core via the global Tauri API
// (withGlobalTauri = true), so no bundler / imports needed.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const REPO_URL = "https://github.com/uxvic/NetCheck";

const el = {
  panel: document.getElementById("panel"),
  state: document.getElementById("state"),
  sub: document.getElementById("sub"),
  globe: document.querySelector("#globe .spin"),
  latency: document.getElementById("latency"),
  down: document.getElementById("down"),
  cardState: document.getElementById("card-state"),
  cardDetail: document.getElementById("card-detail"),
  speedBtn: document.getElementById("speed-btn"),
  signinBtn: document.getElementById("signin-btn"),
  autostart: document.getElementById("autostart"),
  repo: document.getElementById("repo"),
  usage: document.getElementById("usage"),
  usageVal: document.getElementById("usage-val"),
};

let usagePeriod = "today";

const SUBTITLE = {
  green: "Your connection is alive",
  amber: "Heads up — check your connection",
  red: "No real internet right now",
  slate: "Checking your connection…",
};

function spinDuration(downMbps) {
  // livelier with more throughput; clamp so it never stalls or blurs.
  const d = 12 / (1 + (downMbps || 0) / 20);
  return Math.min(12, Math.max(0.8, d));
}

function fmtMbps(v) {
  if (v == null) return "—";
  if (v >= 100) return Math.round(v) + " Mbps";
  return v.toFixed(1) + " Mbps";
}

function fmtBytes(b) {
  if (b == null) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let v = b,
    i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  const n = i === 0 ? b : v < 10 ? v.toFixed(1) : Math.round(v);
  return n + " " + u[i];
}

// Data usage comes from the OS (Windows only). On other platforms the command
// rejects and we just hide the row.
async function loadUsage() {
  try {
    const d = await invoke("get_data_usage", { period: usagePeriod });
    el.usageVal.textContent = fmtBytes(d.totalBytes);
    el.usage.hidden = false;
  } catch (e) {
    el.usage.hidden = true;
  }
}

function render(p) {
  if (!p) return;
  el.panel.className = "tier-" + (p.color || "slate");
  el.state.textContent = p.state || "—";
  el.sub.textContent = SUBTITLE[p.color] || "";
  el.latency.textContent = p.latencyMs != null ? Math.round(p.latencyMs) + " ms" : "—";
  el.down.textContent = fmtMbps(p.downMbps);
  el.globe.style.animationDuration = spinDuration(p.downMbps) + "s";

  el.cardState.textContent = p.state || "—";
  if (p.testMbps != null) {
    el.cardDetail.textContent = "Last speed test: " + fmtMbps(p.testMbps);
  } else if (p.color === "red") {
    el.cardDetail.textContent = "Connected to the network, but no internet";
  } else if (p.color === "amber" && p.tier === "portal") {
    el.cardDetail.textContent = "Looks like a sign-in / captive portal";
  } else {
    el.cardDetail.textContent = "Run a speed test to measure your line";
  }
  // Show the one-tap sign-in shortcut only when we're behind a captive portal.
  el.signinBtn.hidden = p.tier !== "portal";
}

async function runSpeedTest() {
  if (el.speedBtn.disabled) return;
  el.speedBtn.disabled = true;
  el.speedBtn.textContent = "Testing…";
  el.cardDetail.textContent = "Measuring download speed…";
  try {
    const mbps = await invoke("run_speed_test");
    el.cardDetail.textContent = "Download: " + fmtMbps(mbps);
  } catch (e) {
    el.cardDetail.textContent = "Speed test failed — try again";
    console.error(e);
  } finally {
    el.speedBtn.disabled = false;
    el.speedBtn.textContent = "Run speed test";
  }
}

async function init() {
  try {
    render(await invoke("get_current_status"));
  } catch (e) {
    console.error(e);
  }
  await listen("status", (ev) => render(ev.payload));
  await listen("run-speed-test", runSpeedTest);

  el.speedBtn.addEventListener("click", runSpeedTest);
  // neverssl.com is plain HTTP (no HSTS), so on a captive network it forces the portal.
  el.signinBtn.addEventListener("click", () =>
    invoke("open_external", { url: "http://neverssl.com" }).catch(console.error),
  );

  try {
    el.autostart.checked = await invoke("get_autostart");
  } catch (e) {
    console.error(e);
  }
  el.autostart.addEventListener("change", async () => {
    try {
      await invoke("set_autostart", { enable: el.autostart.checked });
    } catch (e) {
      el.autostart.checked = !el.autostart.checked;
      console.error(e);
    }
  });

  el.repo.addEventListener("click", (e) => {
    e.preventDefault();
    invoke("open_external", { url: REPO_URL }).catch(console.error);
  });

  document.querySelectorAll("#usage .u-seg button").forEach((b) => {
    b.addEventListener("click", () => {
      usagePeriod = b.dataset.period;
      document
        .querySelectorAll("#usage .u-seg button")
        .forEach((x) => x.classList.toggle("active", x === b));
      loadUsage();
    });
  });
  loadUsage();
}

init();
