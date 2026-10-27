# Third-party notices

Assets bundled into the server binary or image that are not our own code.
Rust and npm dependencies are not listed here; `cargo deny check licenses`
and `license-checker` enforce the allow-list in `deny.toml` and
`frontend/package.json` on every build.

| Asset | Where | Licence |
|---|---|---|
| PT Serif Regular and Bold (ParaType) | `backend/assets/fonts/PTSerif-*.ttf`, embedded by the cover watermark (`metadata/watermark.rs`) | SIL Open Font License 1.1 — `backend/assets/fonts/OFL.txt` |

Programs the image builds from their release sources (versions and checksums
at the top of `backend/Dockerfile`) and runs as separate processes, not linked
to the server: `ffmpeg` and `ffprobe` from FFmpeg (LGPL-2.1-or-later, built
without its GPL parts) with Chromaprint (LGPL-2.1-or-later) and Debian's LAME
library (LGPL-2.0-or-later); `aubiotrack` from aubio (GPL-3.0-or-later).
