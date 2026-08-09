# Security Policy

SimFleet is an all-in-one companion app for Microsoft Flight Simulator 2020 &
2024. We take the security of the application and its users seriously and
appreciate reports that help keep it safe.

## Supported Versions

Security fixes are provided for the latest released version only. Please make
sure you are running the newest release before reporting an issue.

| Version | Supported          |
| ------- | ------------------ |
| 7.4.0 (latest) | ✅ |
| < 7.4.0 | ❌ |

Download the latest installer from the
[Releases page](https://github.com/Daniiel18/msfs-addons-browser/releases).

## Reporting a Vulnerability

**Please report security-sensitive issues privately** — do not open a public
issue for anything that could put users at risk before a fix is available.

- **Preferred:** open a private
  [GitHub Security Advisory](https://github.com/Daniiel18/msfs-addons-browser/security/advisories/new).
  This keeps the details confidential until a fix is ready.
- **Non-sensitive reports** (hardening suggestions, low-risk findings) can be
  filed as a regular
  [GitHub issue](https://github.com/Daniiel18/msfs-addons-browser/issues).

### What to Include

To help us reproduce and address the issue quickly, please include:

- A clear description of the vulnerability and its potential impact.
- The affected version (e.g. 7.4.0) and your Windows version.
- Step-by-step instructions to reproduce it.
- Any relevant logs, screenshots, or proof-of-concept material.
- The affected component if known (e.g. installer, download browser, cloud
  sync, SimConnect tracking).

Please do not include real credentials, tokens, or personal data in your
report.

### Response Expectations

- **Acknowledgement:** we aim to confirm receipt within **5 business days**.
- **Assessment:** we will investigate and share an initial evaluation, and keep
  you updated on progress toward a fix.
- **Disclosure:** please allow a reasonable window for a fix to ship before any
  public disclosure. We are happy to credit reporters who wish to be named.

This is an independent, community-maintained project, so timelines are
best-effort.

## Data & Privacy

SimFleet is designed to run locally on your machine:

- **Local by default.** App data (catalog cache, logbook, preferences) is
  stored in a portable `data/` folder next to the executable, falling back to
  `%LOCALAPPDATA%\SimFleet\data`.
- **No telemetry.** The app does not collect or transmit usage analytics.
- **Cloud sync is opt-in.** Flight-log and preference sync uses **your own**
  Google OAuth credentials and writes to a private Google Drive
  `appDataFolder`. It is disabled unless you explicitly configure and enable
  it.
- **Third-party access.** Downloads and addon metadata are fetched from
  third-party sites at your request. You are responsible for complying with the
  terms of service of any sites the app accesses and with individual addon
  licenses.

---

SimFleet is an independent, unofficial fan project. It is **not** affiliated
with, endorsed by, or associated with Microsoft, Asobo Studio, Xbox Game
Studios, FSDreamTeam/GSX, flightsim.to, SceneryAddons, SimPlaza, Skybound, or
any addon developer. All trademarks belong to their respective owners.
