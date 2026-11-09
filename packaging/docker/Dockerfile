# Build image for portable Linux packages and the Windows .exe.
# Debian 12 (glibc 2.36) so the Linux binary also runs on older distros.
FROM rust:1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev \
        gcc-mingw-w64-x86-64 \
        zip file desktop-file-utils nsis \
    && apt-get clean

RUN rustup target add x86_64-pc-windows-gnu \
    && cargo install --locked cargo-deb cargo-generate-rpm

# appimagetool (runs without FUSE inside the container)
RUN curl -fsSL -o /usr/local/bin/appimagetool \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage \
    && chmod +x /usr/local/bin/appimagetool
ENV APPIMAGE_EXTRACT_AND_RUN=1

# the build runs as the host user; let it use the toolchain
RUN chmod -R a+rwX /usr/local/cargo /usr/local/rustup
WORKDIR /src
