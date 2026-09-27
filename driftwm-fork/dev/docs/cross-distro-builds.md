# Cross-distro build testing (containers)

Use podman to verify builds and dependency lists on other distros (Docker
Desktop is flaky on Fedora):

```bash
# Arch Linux — run from the repo root
podman run --rm -it --security-opt label=disable -v "$PWD":/src archlinux:latest bash
pacman -Syu --noconfirm rust cargo pkg-config libdisplay-info libinput seatd mesa libxkbcommon
cd /src && cargo build
```

Notes:

- `--security-opt label=disable` is required on Fedora (SELinux blocks libc
  inside the container otherwise).
- Use `cargo build` (not `--release`) — release optimizations are slow and
  unnecessary for verifying deps.
- Don't copy the repo inside the container — `target/` is huge. Mount directly
  without `:ro` so cargo can write to `target/`.
