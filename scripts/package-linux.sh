#!/usr/bin/env bash
# Packages asides for Linux, for this computer's architecture: the app, its overlay and window, the
# `asides` command and the helper that types into terminals.
#
#   scripts/package-linux.sh
#     -> dist/asides-VERSION-linux-ARCH.tar.gz   (the programs and packaging/linux/install.sh: per user)
#        dist/asides_VERSION_DEBARCH.deb         (Debian, Ubuntu, Mint; if dpkg-deb is here)
#        dist/asides-VERSION-1.ARCH.rpm          (Fedora, openSUSE; if rpmbuild is here)
#
# A build of its own, for any computer of the architecture rather than this one. Build on the oldest
# system it should run on (Ubuntu 22.04 in CI: glibc 2.35). Needs Rust, a C++ compiler, CMake, clang
# (whisper.cpp) and the -dev packages CONTRIBUTING.md lists.
#
# No package grants the helper its capability: asides' setup asks, when typing needs it.
#
# ASIDES_PROFILE=dev packages a debug build (CI checks the packaging so).
set -euo pipefail
umask 022
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
profile=${ASIDES_PROFILE:-release}
out=$([ "$profile" = dev ] && echo debug || echo "$profile")
arch=$(uname -m)
case "$arch" in
x86_64)
    debarch=amd64
    # whisper.cpp for a PC of the last ten years, not for this one.
    export GGML_NATIVE=OFF GGML_AVX=ON GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON
    ;;
aarch64)
    debarch=arm64
    export GGML_NATIVE=OFF
    ;;
*)
    echo "asides isn't packaged for $arch" >&2
    exit 1
    ;;
esac
programs=(asides asides-app asides-overlay asides-keys)
target=${CARGO_TARGET_DIR:-$repo/target}/linux-package
CARGO_TARGET_DIR=$target cargo build --profile "$profile" -p asides-app -p asides-overlay -p asides-cli -p asides-keys
built=$target/$out

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
dist=${ASIDES_DIST:-$repo/dist}
mkdir -p "$dist"

# --- The archive ---
name=asides-$version-linux-$arch
mkdir -p "$work/$name"
for program in "${programs[@]}"; do
    install -m755 "$built/$program" "$work/$name/$program"
done
[ "$profile" = dev ] || strip "${programs[@]/#/$work/$name/}"
install -m644 packaging/linux/asides.desktop packaging/linux/asides.png "$work/$name/"
install -m755 packaging/linux/install.sh "$work/$name/"
tar -C "$work" -czf "$dist/$name.tar.gz" "$name"

# --- What a system package installs ---
root=$work/root
mkdir -p "$root/usr/lib/asides" "$root/usr/bin" "$root/usr/share/applications" \
    "$root/usr/share/icons/hicolor/256x256/apps"
for program in "${programs[@]}"; do
    install -m755 "$work/$name/$program" "$root/usr/lib/asides/$program"
done
ln -s ../lib/asides/asides "$root/usr/bin/asides"
sed 's|@EXEC@|/usr/lib/asides/asides-app|' packaging/linux/asides.desktop \
    >"$root/usr/share/applications/io.github.mihaicristianfarcas.asides.desktop"
install -m644 packaging/linux/asides.png "$root/usr/share/icons/hicolor/256x256/apps/asides.png"
size=$(du -sk "$root" | cut -f1)

# --- .deb ---
if command -v dpkg-deb >/dev/null; then
    deb=$work/deb
    cp -a "$root" "$deb"
    mkdir -p "$deb/DEBIAN"
    cat >"$deb/DEBIAN/control" <<CONTROL
Package: asides
Version: $version
Architecture: $debarch
Maintainer: Mihai-Cristian Farcaș <146859576+mihaicristianfarcas@users.noreply.github.com>
Installed-Size: $size
Depends: libc6 (>= 2.35), libstdc++6, libgcc-s1, libasound2t64 | libasound2, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1, mesa-vulkan-drivers, fontconfig, libcap2-bin
Recommends: xwayland, pkexec | policykit-1
Homepage: https://github.com/mihaicristianfarcas/asides
Section: devel
Priority: optional
Description: Freeze the screen, click what's wrong, say why
 asides freezes every screen under a small pill. Pin what's wrong, measure,
 draw and talk; it's transcribed locally and typed into your coding agent's
 session as your own message, with screenshots.
CONTROL
    dpkg-deb --root-owner-group --build "$deb" "$dist/asides_${version}_$debarch.deb" >/dev/null
fi

# --- .rpm ---
if command -v rpmbuild >/dev/null; then
    rpm=$work/rpm
    mkdir -p "$rpm/SPECS" "$rpm/RPMS"
    cat >"$rpm/SPECS/asides.spec" <<SPEC
Name: asides
Version: $version
Release: 1
Summary: Freeze the screen, click what's wrong, say why
License: MIT OR Apache-2.0
URL: https://github.com/mihaicristianfarcas/asides
Requires: fontconfig
Requires: (vulkan-loader or libvulkan1)
Recommends: (mesa-vulkan-drivers or libvulkan_lvp)
Recommends: (xorg-x11-server-Xwayland or xwayland)
Recommends: polkit
%global debug_package %{nil}
%global __strip /bin/true
%global _build_id_links none

%description
asides freezes every screen under a small pill. Pin what's wrong, measure, draw and talk; it's
transcribed locally and typed into your coding agent's session as your own message, with screenshots.

%install
cp -a "$root/." %{buildroot}/

%files
/usr/lib/asides
/usr/bin/asides
/usr/share/applications/io.github.mihaicristianfarcas.asides.desktop
/usr/share/icons/hicolor/256x256/apps/asides.png
SPEC
    rpmbuild -bb --quiet --define "_topdir $rpm" --define "_rpmdir $rpm/RPMS" \
        --define "_rpmfilename %%{NAME}-%%{VERSION}-%%{RELEASE}.%%{ARCH}.rpm" "$rpm/SPECS/asides.spec"
    cp "$rpm/RPMS/"*.rpm "$dist/"
fi

ls -l "$dist" | grep -E "asides[-_]$version.*(linux|\.deb|\.rpm)"
