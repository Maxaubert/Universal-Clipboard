# Headless display servers for the live clipboard tests of uc-clip-linux.
# Arch has a current Sway (ext-data-control-v1 and wlr-data-control), Xvfb, xclip and wl-clipboard.
FROM archlinux:latest
RUN pacman -Syu --noconfirm --needed base-devel sway xorg-server-xvfb wl-clipboard xclip rustup \
 && pacman -Scc --noconfirm  && (setcap -r /usr/bin/sway || true)
# Arch gives sway the cap_sys_nice file capability; a container without it cannot exec sway.
RUN rustup toolchain install 1.99.0 --profile minimal -c clippy,rustfmt && rustup default 1.99.0
WORKDIR /src
