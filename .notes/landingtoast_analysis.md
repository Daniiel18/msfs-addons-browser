# LandingToast Analysis (v1.1.5)

**Source:** `D:\Downloads\LandingToast-1.1.5_qXJz4\LandingToast\LandingToast.exe`

**Method:** Extracted .NET single-file bundle with `sfextract`, then string-mined
`LandingToast.dll` (the actual managed assembly inside the bundle).

## SimConnect Variables Subscribed

Confirmed via string extraction from the managed DLL:

| Variable | Unit | Notes |
|---|---|---|
| `SIM ON GROUND` | bool | Touchdown trigger (flank 0→1) |
| `VERTICAL SPEED` | feet per minute | **The actual FPM source** — NOT touchdown simvar |
| `RADIO HEIGHT` | feet | AGL — likely used to gate the capture |
| `G FORCE` | number | Impact severity |
| `PLANE BANK DEGREES` | (radians) | Wing-level at touchdown |
| `PLANE PITCH DEGREES` | (radians) | Pitch attitude at touchdown |
| `PLANE HEADING DEGREES TRUE` | degrees | For runway alignment delta |
| `AMBIENT WIND DIRECTION` | degrees | Headwind/crosswind calc |
| `AMBIENT WIND VELOCITY` | knots | Wind component at landing |

## Critical Insight

LandingToast does **NOT** subscribe to `PLANE TOUCHDOWN NORMAL VELOCITY`.
They read live `VERTICAL SPEED` at **SIM_FRAME period** (60Hz typically).

The reasoning: at 60Hz, the polling interval (~16ms) is faster than the
touchdown event itself. When `SIM ON GROUND` flips to 1, the current
frame's `VERTICAL SPEED` IS the touchdown rate.

## Implication for SimFleet P7.6b

My previous attempt (P7.5) used `PLANE TOUCHDOWN NORMAL VELOCITY` + 3s post-
touchdown observation. That fails when third-party aircraft don't update
that simvar correctly.

**New design:**

- **Stream B** (separate from main 4Hz stream): subscribe to
  - `SIM ON GROUND`
  - `VERTICAL SPEED`
  - `RADIO HEIGHT`
  - `G FORCE`
  - `PLANE BANK DEGREES`
  - `PLANE PITCH DEGREES`
- **Period:** `SIMCONNECT_PERIOD_SIM_FRAME`
- **Flag:** `SIMCONNECT_DATA_REQUEST_FLAG_CHANGED` (only fire when something
  changes — minimal CPU overhead)
- **Activation gate:** only enable when `phase == Airborne` AND
  `radio_height_ft < 500` (final approach). Disable on touchdown +5s.
- **Detection:** keep last frame's `SIM ON GROUND`; when previous=0 and
  current=1, capture current frame's `VERTICAL SPEED` directly as FPM.
- **No `PLANE TOUCHDOWN NORMAL VELOCITY`** — redundant.

## Unrelated finds in the binary

- Uses ScreenRecorderLib for landing GIF capture
- Bundle target: .NET 6.0
- 49 files extracted including Windows runtime DLLs
