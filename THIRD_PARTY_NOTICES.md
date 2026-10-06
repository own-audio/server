# Third-party notices

Assets bundled into the server binary or image that are not our own code.
Rust and npm dependencies are not listed here; `cargo deny check licenses`
and `license-checker` enforce the allow-list in `deny.toml` and
`frontend/package.json` on every build.

| Asset | Where | Licence |
|---|---|---|
| PT Serif Regular and Bold (ParaType) | `backend/assets/fonts/PTSerif-*.ttf`, embedded by the cover watermark (`metadata/watermark.rs`) | SIL Open Font License 1.1 — `backend/assets/fonts/OFL.txt` |

Programs the image installs from Debian and runs as separate processes
(not linked): `ffmpeg` (LGPL/GPL), `yt-dlp` (Unlicense), `aubio` (GPL-3.0).
