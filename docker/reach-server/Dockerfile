# Noble Outreach dedicated server (reach-host.exe --dedicated) under Wine.
# The engine renders through D3D11, so: Wine + DXVK (D3D11 -> Vulkan) inside a hidden Xvfb display. Put
# reach-host.exe (from a release, or your own build) next to this Dockerfile before building. Your MCC install is
# mounted read-only at /mcc; no game files go into the image.
#
# GPU=cpu (default): no GPU needed. Vulkan runs on Mesa's software driver (lavapipe) and the server starts with
# --no-render, so the engine only creates its textures and shaders while a map loads and never draws. Same CPU
# use as on a GPU (about a third of one core with players in).
# GPU=nvidia: the host's NVIDIA GPU (compose.nvidia.yml). Some NVIDIA hosts only expose the compute libraries to
# containers (no Vulkan/GLX), so that stage takes the graphics libraries from NVIDIA's own installer of the SAME
# driver version as the host kernel module: NVIDIA_VERSION = `nvidia-smi --query-gpu=driver_version
# --format=csv,noheader`.
ARG GPU=cpu

FROM debian:trixie-slim AS nvidia
ARG NVIDIA_VERSION=580.105.08
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates xz-utils \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /tmp
RUN curl -fsSLo nv.run "https://download.nvidia.com/XFree86/Linux-x86_64/${NVIDIA_VERSION}/NVIDIA-Linux-x86_64-${NVIDIA_VERSION}.run" \
 && sh nv.run --extract-only --target nv >/dev/null && rm nv.run
RUN mkdir -p /out/lib /out/icd \
 && cd nv \
 && for l in libGLX_nvidia libnvidia-glcore libnvidia-glvkspirv libnvidia-gpucomp libnvidia-tls libnvidia-eglcore \
             libnvidia-glsi libEGL_nvidia libnvidia-allocator; do \
      [ -f "$l.so.${NVIDIA_VERSION}" ] && cp "$l.so.${NVIDIA_VERSION}" /out/lib/ || true; \
    done \
 && printf '{"file_format_version":"1.0.1","ICD":{"library_path":"libGLX_nvidia.so.0","api_version":"1.4.312"}}\n' \
      > /out/icd/nvidia_icd.json \
 && ls /out/lib && (ls *.json* || true)

FROM debian:trixie-slim AS dxvk
ARG DXVK_VERSION=2.6.1
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL "https://github.com/doitsujin/dxvk/releases/download/v${DXVK_VERSION}/dxvk-${DXVK_VERSION}.tar.gz" \
    | tar -xz -C /opt && mv /opt/dxvk-${DXVK_VERSION} /opt/dxvk

FROM debian:trixie-slim AS base
ENV DEBIAN_FRONTEND=noninteractive
# 64-bit Wine only (reach-host is 64-bit); libvulkan1 = Vulkan loader; xvfb = the hidden display.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      wine wine64 libvulkan1 xvfb xauth ca-certificates tini procps \
 && rm -rf /var/lib/apt/lists/*
COPY --from=dxvk /opt/dxvk /opt/dxvk
# No sound card: without an audio device XAudio2 hands bink a null voice and the level unload after the
# last player leaves crashes. ALSA's null plugin as the default device gives Wine a sink that discards audio.
RUN printf 'pcm.!default { type null }\nctl.!default { type hw card 0 }\n' > /etc/asound.conf

# No GPU: Mesa's lavapipe (software Vulkan) and no rendering.
FROM base AS gpu-cpu
RUN apt-get update \
 && apt-get install -y --no-install-recommends mesa-vulkan-drivers \
 && rm -rf /var/lib/apt/lists/*
ENV VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
    VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json \
    REACH_RENDER=off

FROM base AS gpu-nvidia
ARG NVIDIA_VERSION=580.105.08
COPY --from=nvidia /out/lib/ /usr/lib/x86_64-linux-gnu/
COPY --from=nvidia /out/icd/nvidia_icd.json /usr/share/vulkan/icd.d/nvidia_icd.json
RUN cd /usr/lib/x86_64-linux-gnu \
 && ln -sf libGLX_nvidia.so.${NVIDIA_VERSION} libGLX_nvidia.so.0 \
 && { [ -f libEGL_nvidia.so.${NVIDIA_VERSION} ] && ln -sf libEGL_nvidia.so.${NVIDIA_VERSION} libEGL_nvidia.so.0 || true; } \
 && ldconfig
ENV NVIDIA_DRIVER_CAPABILITIES=all \
    REACH_RENDER=on

FROM gpu-${GPU}
ARG UID=1000
ARG GID=1000
RUN groupadd -o -g ${GID} game && useradd -o -m -u ${UID} -g ${GID} game

COPY reach-host.exe /opt/reach/reach-host.exe
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

USER game
ENV WINEPREFIX=/data/prefix \
    WINEARCH=win64 \
    WINEDEBUG=-all \
    WINEDLLOVERRIDES="d3d11,dxgi,d3d10core=n,b;mscoree,mshtml=" \
    DXVK_LOG_LEVEL=warn \
    DXVK_FRAME_RATE=60 \
    DXVK_STATE_CACHE_PATH=/data

EXPOSE 47800/udp
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
