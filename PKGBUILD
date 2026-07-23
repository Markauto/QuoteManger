# Local-source package recipe. Run `makepkg -si` from the repository root.

pkgname=quotes
pkgver=0.1.0
pkgrel=1
pkgdesc='Portable SQLite-backed quote manager with a terminal UI'
arch=('x86_64')
url=''
# The project does not currently declare an upstream license.
license=('LicenseRef-Unknown')
depends=('glibc' 'libgcc')
makedepends=('cargo')
# Arch's GCC LTO objects cannot be consumed by rust-lld when SQLite is bundled.
options=('!lto')
source=()
b2sums=()

_source_dir="$pkgname-$pkgver"
_project_root="$startdir"

# The default makepkg build directory is ./src, which is real project source
# here. Keep packaging work isolated unless the user configured another path.
if [[ "$BUILDDIR" -ef "$startdir" ]]; then
  BUILDDIR="$startdir/.makepkg"
fi

prepare() {
  local build_root="$srcdir/$_source_dir"

  rm -rf -- "$build_root"
  install -d "$build_root"
  cp -a \
    "$_project_root/Cargo.lock" \
    "$_project_root/Cargo.toml" \
    "$_project_root/Quotes" \
    "$_project_root/README.md" \
    "$_project_root/rust-toolchain.toml" \
    "$_project_root/src" \
    "$_project_root/tests" \
    "$build_root/"

  export RUSTUP_TOOLCHAIN=stable
  export CARGO_HOME="$srcdir/cargo-home"
  cd "$build_root"
  cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR="$srcdir/target"
  cd "$srcdir/$_source_dir"
  cargo build --frozen --release --all-features
}

check() {
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR="$srcdir/target"
  cd "$srcdir/$_source_dir"
  cargo test --frozen --all-features
}

package() {
  install -Dm0755 "$srcdir/target/release/quotes" "$pkgdir/usr/bin/quotes"
  install -Dm0644 "$srcdir/$_source_dir/README.md" \
    "$pkgdir/usr/share/doc/quotes/README.md"
}
