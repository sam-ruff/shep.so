FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
ARG SHEP_UID=1000
ARG SHEP_GID=1000
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git build-essential cmake pkg-config perl unzip xz-utils \
    libssl-dev libdbus-1-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libgl1-mesa-dev \
    libvulkan1 mesa-vulkan-drivers libgl1-mesa-dri libegl1 \
    python3 python3-pil python3-gi python3-dbus gir1.2-gtk-3.0 \
    xvfb xauth x11-utils xdotool zenity xclip dbus dbus-x11 dbus-user-session systemd \
    gnome-shell ubuntu-session gnome-settings-daemon \
    gnome-shell-extension-ubuntu-dock gnome-shell-extension-appindicator \
    gjs gir1.2-notify-0.7 libgtk-3-0t64 libnss3 libasound2t64 \
    libatk-bridge2.0-0t64 libdrm2 libgbm1 libcups2t64 \
    poppler-utils imagemagick fonts-dejavu-core fonts-liberation fonts-ubuntu \
    fonts-noto-core fonts-noto-color-emoji desktop-file-utils xdg-utils \
    procps psmisc util-linux locales \
    && rm -rf /var/lib/apt/lists/*

# Versioned upstream downloads are verified before execution or extraction.
RUN curl --fail --location --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 180 --retry 2 \
      https://static.rust-lang.org/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init -o /tmp/rustup-init \
    && echo 'dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71  /tmp/rustup-init' | sha256sum --check --strict \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain 1.96.0 --component rustfmt --component clippy \
    && chmod -R a+rX /opt/rustup /opt/cargo \
    && rm /tmp/rustup-init

RUN curl --fail --location --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 180 --retry 2 \
      https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-x64.tar.xz -o /tmp/node.tar.xz \
    && echo 'fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6  /tmp/node.tar.xz' | sha256sum --check --strict \
    && tar -xJf /tmp/node.tar.xz --strip-components=1 -C /usr/local \
    && rm /tmp/node.tar.xz

# Ubuntu's chromium package delegates to snap, which is unsuitable here.
RUN curl --fail --location --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 180 --retry 2 \
      https://storage.googleapis.com/chrome-for-testing-public/153.0.8010.36/linux64/chrome-linux64.zip -o /tmp/chrome.zip \
    && echo '167a098c4fdec156b58a9f678c90a84f9072d789f9c6e7b35496a6987b8b7ef8  /tmp/chrome.zip' | sha256sum --check --strict \
    && unzip -q /tmp/chrome.zip -d /opt \
    && ln -s /opt/chrome-linux64/chrome /usr/local/bin/google-chrome \
    && rm /tmp/chrome.zip

RUN if id ubuntu >/dev/null 2>&1; then userdel --remove ubuntu; fi \
    && if ! getent group "$SHEP_GID" >/dev/null; then groupadd --gid "$SHEP_GID" shep; fi \
    && useradd --uid "$SHEP_UID" --gid "$SHEP_GID" --create-home shep \
    && touch /etc/shep-desktop-ci
ENV HOME=/home/shep LANG=C.UTF-8 LC_ALL=C.UTF-8
ENV CARGO_BUILD_JOBS=4 CARGO_HOME=/workspace/artifacts/ci/cargo
USER shep
WORKDIR /workspace
