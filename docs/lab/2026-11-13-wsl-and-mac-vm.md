# WSL and Mac VM checks (2026-11-13)

Read-only checks for the test lab (plan section 6.0). Only change made: WSL Ubuntu was started.

## 1. WSL Ubuntu on the PC

| Check | Result |
|---|---|
| Distro | Ubuntu 26.04.1 LTS, WSL2 (docker-desktop distro also exists) |
| Kernel | 6.18.33.2-microsoft-standard-WSL2 |
| Resources | 32 CPUs, 30 GB RAM visible |
| WireGuard | Yes. `CONFIG_WIREGUARD=m`, module loads with modprobe, `ip link add wgtest type wireguard` works |
| Network namespaces | Yes. `ip netns add/list/del` works |
| Unprivileged user namespaces | Yes. `unshare -Ur true` works |
| sudo | Default user is root (uid 0), so no password prompt |
| WSLg | Present. Version 1.0.73.2, `DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0` |

Result: WSL can host the Linux lab node and WireGuard/netns network simulation without extra setup.

## 2. macOS VM on the Mac (ssh mac)

| Check | Result |
|---|---|
| Host | macOS 27.2 (26B123), arm64, 12 CPUs, 24 GB RAM, `kern.hv_support=1` |
| Free disk | 153 GiB free on 926 GiB |
| Homebrew | 7.0.13 at /opt/homebrew/bin/brew |
| Rust / Node | Not installed yet |
| tart | NOT installed (see below) |

Findings:
- Tart moved from cirruslabs to OpenAI. `brew install cirruslabs/cli/tart` fails: the tap now maps tart to `openai/tools` (see `tap_migrations.json`). Latest release is 2.41.1 (2026-10-15) at github.com/openai/tart.
- Install needs `brew tap openai/tools`, `brew trust openai/tools` and `brew install openai/tools/tart`. The auto-mode safety classifier denied this command chain (it adds and trusts a third-party tap). It was not retried or worked around.
- Images on ghcr.io are public and anonymous pull works. `cirruslabs/macos-tahoe-base` (macOS 26) and `cirruslabs/macos-sequoia-base` exist with tag `latest`. There is NO public `macos-27-*` image. The closest guest is macOS 26 Tahoe, which is fine on a macOS 27 host.
- A base image is about 25 GB, so disk is enough.

Status: not feasible without one approval. Max (or a session with an allow rule) must approve the tap trust and install. Then:
`tart clone ghcr.io/cirruslabs/macos-tahoe-base:latest uc-mac`, `tart run --no-graphics uc-mac &`, `tart ip uc-mac`, ssh as `admin`/`admin` (image default; change it).
