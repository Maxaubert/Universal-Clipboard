# Live tests for uc-clip-linux

These tests use real, headless display servers: Xvfb (X11) and Sway with
`WLR_BACKENDS=headless` (Wayland data-control). They never use a desktop session.

- `docker.sh`: run from any machine with Docker. Builds an Arch image (current Sway, so both
  `ext-data-control-v1` and `wlr-data-control` are tested), runs clippy and the live tests.
- `run.sh`: starts Xvfb on `:99` and Sway, then runs `cargo test --test live`. Use it only in
  Docker or CI. It unsets `DISPLAY` and `WAYLAND_DISPLAY` first.

The tests skip unless `UCLIP_LIVE_X11` or `UCLIP_LIVE_WAYLAND` is set, and they always skip
inside WSLg, because WSLg bridges to the Windows clipboard.
