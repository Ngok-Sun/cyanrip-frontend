# cyanrip-frontend

A native GTK4 / libadwaita frontend for [cyanrip](https://github.com/cyanreg/cyanrip), written in Rust.

**English** · [中文](README.md)

Status: **v1.0** (2026-11-08). All milestones (M1–M4) delivered and verified on real discs.
Design notes (Chinese): [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md).

## Why another ripper GUI?

`cyanrip` already does bit-perfect extraction with AccurateRip verification, but it is CLI-only.
This project wraps it instead of reimplementing CD reading, so every rip produces exactly the same
EAC CRC32 and AccurateRip results as the command line — while adding disc detection, track selection,
pre-rip metadata lookup, cover art, tagging and a queue-free single-window workflow.

## Layout

```
crates/cyanrip-core/   # disc probing, rip orchestration, progress/log parsing (no GTK)
crates/cyanrip-cli/    # headless entry: probe / parse-log / rip / match / tag / eject
crates/cyanrip-gui/    # GTK4 + libadwaita UI (Disc / Progress / Result / Settings)
```

## Requirements

- Rust stable (rustup)
- Runtime: `cyanrip` ≥ 0.9.4, `eject` (util-linux), `metaflac`/`flac`
- Optional: `fpcalc` (AcoustID fingerprinting), `cdrdao` (ISRC reading)
- Building the GUI: `libgtk-4-dev`, `libadwaita-1-dev`

## Build and test

```bash
cargo build
cargo test          # 53 tests (streaming/cancel, archive templates, i18n coverage), all offline
```

## Install

```bash
cargo build --release -p cyanrip-gui
install -Dm755 target/release/cyanrip-frontend ~/.local/bin/cyanrip-frontend
install -Dm644 assets/io.github.cyanrip.Frontend.svg \
        ~/.local/share/icons/hicolor/scalable/apps/io.github.cyanrip.Frontend.svg
install -Dm644 assets/io.github.cyanrip.Frontend.desktop \
        ~/.local/share/applications/io.github.cyanrip.Frontend.desktop
gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor
```

The desktop file name must match the application ID (`io.github.cyanrip.Frontend`) or the dock
will show a generic icon.

## Features

- **Disc page** — drive dropdown with vendor/model, refresh (rescan + TOC + metadata), eject,
  read-offset spin with a "Detect offset" action (result remembered per drive model), select
  all/none, per-track checkboxes. Inserting a disc re-probes automatically within 1.5 s.
- **HDCD** — a "This disc is HDCD" checkbox next to the start button. Off by default; when checked
  cyanrip is invoked with `-H` and writes 24-bit FLAC (≈50 % larger). cyanrip does not detect HDCD
  itself, so this stays a user decision.
- **Pre-rip identification** — as soon as a disc is probed, MusicBrainz (Disc ID + MCN) and Discogs
  are queried; the track list shows real titles before ripping. A candidate dropdown switches
  pressings; the selected metadata is passed to cyanrip via `-a`/`-t` so directory and file names
  are right the first time.
- **Cover art** — 180×180 preview on the disc page. Chain: MusicBrainz/CAA → Discogs → Google
  Custom Search (optional, needs API key + CX). Scrapers can be toggled individually; with none
  enabled no image request is made at all. The same image is embedded into FLAC and saved as
  `cover.jpg` in the album folder.
- **CUE import/export** — discs missing from every database can import a CUE sheet (UTF-8 or GBK).
  The current selection can be exported as a CUE (per-track INDEX times) plus the cover image.
- **Progress** — two progress bars plus a single terminal-style status line
  (`Ripping track 5, progress - 47.32%, ETA - 2m · 2.3x · elapsed 12:40`). The log pane only keeps
  real log lines, never `\r` progress spam.
- **Result** — verdict line, overview (device, offset, C2, AR ID, error count) and a per-track table
  with EAC CRC32, AR v1/v2 checksums, confidence and status.
- **Metadata & tagging** — after a rip the result page offers candidates (MusicBrainz → Discogs →
  AcoustID); "Write tags" archives to `Artist/Album/`, writes Vorbis comments and embeds the cover.
- **Settings** — Device & drive / Output (folder picker) / Metadata (Discogs token stored in
  `secrets.toml`, mode 0600) / Behaviour (sound picker + preview, eject, notification, shutdown) /
  Advanced (build time) / **Apply** (explicit save + restore defaults).

## Dual drives

A second launcher allows two instances (independent config, both may run):

```bash
cyanrip-frontend-2   # CYANRIP_GUI_STANDALONE=1 + its own config file
```

Each instance picks its own drive; device, offset and output root are independent. "Shut down when
done" waits for the last instance to finish.

Double-CD sets can be ripped in parallel: identification matches the medium by track count, output
directories become `Album CD1 [FLAC]` / `Album CD2 [FLAC]`, tags carry `DISCNUMBER`/`DISCTOTAL`
and files get `CD1-`/`CD2-` prefixes.

## Archive templates

Settings → Output controls directory/file/log templates, applied when archiving:

- Fields: `{artist}` `{album}` `{year}` `{title}` `{track}` `{track:02}` `{disc}` `{disc_total}` `{track_total}`
- Defaults: directory `{artist}/{album}`, file `{track:02} {title}`, log `{album}`
- Multi-disc sets get a `CDn-` prefix unless the file template already contains `{disc}`
- Values are sanitised; a directory template containing `..` is rejected

## Keyboard shortcuts

`Ctrl+R` start ripping · `Ctrl+E` eject · `F5` refresh · `Ctrl+Q` quit

## UI language

Settings → Behaviour → **Interface language**: Follow system / 中文 / English. Written to
`ui.language`; **takes effect after restarting** (gettext locale is read once at startup).

Source strings use Chinese as msgids; `po/en.po` holds the English translation:

```bash
po/update.sh          # xgettext → msgmerge → msgfmt → install .mo
```

## Self-test switches

| Variable | Purpose |
|---|---|
| `CYANRIP_GUI_SMOKE=1` | build + probe, exit after 2 s |
| `CYANRIP_GUI_AUTORIP=1` | rip the first selected track, print `AUTORIP_OK n/m` |
| `CYANRIP_GUI_SELFTEST=1` | assert save flow, select-all, device dropdown |
| `CYANRIP_GUI_EJECTTEST=1` | click eject, verify tray state, close again |
| `CYANRIP_GUI_STANDALONE=1` | allow a second instance |
| `CYANRIP_GUI_DEBUG=1` | print media transitions and metadata progress |
| `CYANRIP_GUI_AUTOAPPLY=1` | auto-write tags once candidates arrive (testing) |
| `CYANRIP_GUI_FAKE_META=1` | synthetic candidate instead of network lookup |
| `CYANRIP_GUI_ROOT=<dir>` | temporary output root (keeps your library clean) |
| `CYANRIP_DISCOGS_TOKEN` / `CYANRIP_ACOUSTID_KEY` | metadata credentials |

## CLI

```bash
cargo run -p cyanrip-cli -- probe --device /dev/sr0
cargo run -p cyanrip-cli -- parse-log crates/cyanrip-core/tests/fixtures/log_quartet.txt
cargo run -p cyanrip-cli -- rip --device /dev/sr0 --offset 48 -- -Z 2 -P 3 -o flac -N
cargo run -p cyanrip-cli -- match --device /dev/sr0 --json
cargo run -p cyanrip-cli -- tag --dir <rip dir> --root ~/Music
```

## Verified on hardware

- Three real log fixtures (Joe Hisaishi *Quartet* 13/13, 花儿乐队 CD1 15/16, CD2 8/8) parse
  identically to cyanrip's own output.
- Fake-cyanrip streaming test: progress, `TrackStarted`, `TrackDone`, `RipFinished`.
- Cancel path: the child process is terminated within 300 ms and `Cancelled` is emitted.
- **Real disc** (Joe Hisaishi *Quartet*, PIONEER DVR-106D, offset +48):
  `probe` reports 13 tracks / 41:22 / lead-out 186178; `rip -l 5 -Z 2` yields
  `EAC CRC32 7FF4B823`, `AR v2 539F7C13 (confidence 8)` — bit-identical to a full cyanrip run.
- Dual-drive concurrent rip: both discs finished, both AccurateRip-verified, outputs separated by
  Disc ID.
- HDCD disc: with the checkbox enabled the log reports `HDCD detected: yes` and the FLAC is 24-bit.

## Known pitfalls

- `CDROMREADTOCHDR` is **0x5305**, not 0x5302 (which is `CDROMRESUME`). The wrong value returns 0
  and writes nothing — a silent failure this project hit once.
- On this machine the drive sits behind a SiI3811 bridge: `CDROMEJECT` is rejected
  (`0x53/0x02 MEDIUM REMOVAL PREVENTED`) yet returns 0, so eject goes through util-linux `eject`
  (SCSI `START STOP UNIT`).
- `AdwApplicationWindow` does not support `set_titlebar`; the header bar must live in
  `adw::ToolbarView`.
- `AdwActionRow` uses Pango markup by default; dynamic titles must set `use_markup(false)` or
  titles containing `&` render blank.
- Offsets measured with `cyanrip -f` must be re-verified with a real rip + AccurateRip: on a second
  drive it reported +488 (confidence 1) while +500 gave AR v2 confidence 15.
- Some drives accept `-S` (speed) and ignore it. On the two drives here, 4x/16x/default all took
  the same time, so speed selection is not exposed in the UI.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).

`cyanrip` itself is a separate GPL-2.0-or-later program invoked as a subprocess; no code is linked.
