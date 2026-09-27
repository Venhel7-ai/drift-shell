pkgname=drift-shell
pkgver=0.1.0
pkgrel=1
pkgdesc='Experimental Drift Shell V3 implementation; see STATUS-RU.md'
arch=('x86_64' 'aarch64')
license=('GPL-3.0-or-later')
depends=('python' 'quickshell' 'libinput' 'libseat' 'libdisplay-info' 'libxkbcommon' 'mesa' 'fontconfig' 'wayland' 'glib2' 'pam' 'foot' 'inter-font' 'ttf-jetbrains-mono')
makedepends=('rust' 'pkgconf' 'git')
optdepends=('xwayland-satellite: X11 applications' 'pyside6: portable UI preview' 'greetd: experimental graphical login')
options=('!strip')

build() {
  cd "$startdir/driftwm-fork"
  cargo build --release --locked
}
check() {
  cd "$startdir"
  PYTHONPATH="$startdir/shell-core:$startdir/vendor" python -m unittest discover -s tests -v
  "$startdir/driftwm-fork/target/release/driftwm" --config "$startdir/packaging/arch/driftwm.toml" --check-config
}
package() {
  install -Dm755 "$startdir/driftwm-fork/target/release/driftwm" "$pkgdir/usr/bin/driftwm-shell"
  install -d "$pkgdir/usr/share/drift-shell"
  for dir in shell-core shell-ui shell-lock shell-greeter themes schemas vendor preview zen-integration; do
    cp -a "$startdir/$dir" "$pkgdir/usr/share/drift-shell/"
  done
  find "$pkgdir/usr/share/drift-shell" -name __pycache__ -type d -prune -exec rm -rf {} +
  install -Dm644 "$startdir/packaging/arch/driftwm.toml" "$pkgdir/usr/share/drift-shell/driftwm.toml"
  for script in "$startdir"/bin/*; do install -Dm755 "$script" "$pkgdir/usr/bin/$(basename "$script")"; done
  for service in "$startdir"/systemd/*.service; do install -Dm644 "$service" "$pkgdir/usr/lib/systemd/user/$(basename "$service")"; done
  install -Dm644 "$startdir/packaging/arch/drift-shell.desktop" "$pkgdir/usr/share/wayland-sessions/drift-shell.desktop"
  install -Dm644 "$startdir/packaging/arch/pam" "$pkgdir/etc/pam.d/drift-shell"
  install -Dm644 "$startdir/README-RU.md" "$pkgdir/usr/share/doc/drift-shell/README-RU.md"
  install -Dm644 "$startdir/STATUS-RU.md" "$pkgdir/usr/share/doc/drift-shell/STATUS-RU.md"
  install -Dm644 "$startdir/driftwm-fork/LICENSE" "$pkgdir/usr/share/licenses/drift-shell/LICENSE"
}
