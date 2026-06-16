# NetCheck for Windows

A tiny system-tray app that tells you, at a glance, whether your internet is
*actually* alive — not just whether Wi-Fi says "connected." It catches the case
the Wi-Fi icon lies about: link up, but no real internet (a dead VPN, a captive
portal). The tray icon is a living globe that **changes colour with status** and
spins faster the more data is flowing.

This is the Windows companion to the macOS app
([uxvic/NetCheck](https://github.com/uxvic/NetCheck)). The concept and the
living-globe design are shared; the code is a separate native build — **Tauri 2
(Rust + the system WebView2)**, not a port of the Swift/AppKit app (none of that
runs on Windows).

## What it does (v0.1)

- **Living globe in the tray** — green = alive, amber = slow or a sign-in wall,
  red = no real internet, slate = checking.
- **Real reachability check** — an actual HTTPS probe (with a TCP fallback), so a
  dead VPN reads as "Offline / Sign-in needed," not "Connected."
- **Latency** — TCP round-trip to `1.1.1.1`, in milliseconds.
- **Live throughput** — current down/up rate from the OS interface counters;
  drives how fast the globe spins.
- **On-demand speed test** — a one-shot Cloudflare download.
- **Click the tray** for a dark flyout with the globe, the numbers, and a
  status-coloured card. Optional launch-at-login.

Deferred to a later version: data-usage tracking and Sparkle-style auto-update
(the macOS app has both).

## Build

Built entirely on GitHub Actions — no local Windows machine needed.

1. Generate icons once (already committed): `python3 scripts/make_icons.py`.
2. Push a tag: `git tag v0.1.0 && git push --tags`.
3. The [`release`](.github/workflows/release.yml) workflow builds on
   `windows-latest` and attaches an NSIS `-setup.exe` and an `.msi` to a **draft**
   GitHub Release. Review, then publish.

To build locally on a Windows box instead:
`cargo install tauri-cli --version "^2" --locked && cargo tauri build`
(run inside `src-tauri/`).

## Install warning (honest note)

The installer is **not yet code-signed**, so Windows SmartScreen will show
"Windows protected your PC." Click **More info → Run anyway**. The app is fully
functional; only the reputation warning differs. Signing (Azure Trusted Signing
is the cheap, CI-friendly path) can be added later without changing anything else.

## Layout

```
dist/                     static flyout UI (HTML/CSS/JS)
src-tauri/
  src/main.rs             tray, status classification, monitor loop, commands
  src/net.rs              reachability / latency / throughput / speed test
  tauri.conf.json         app + bundle config
  capabilities/           v2 permission grants
  icons/                  generated app .ico + coloured tray globes
scripts/make_icons.py     regenerates the icon set from the living-globe design
.github/workflows/        Windows build + release
```
