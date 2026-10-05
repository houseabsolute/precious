FROM rust:latest

RUN apt-get update \
    && apt-get install -y curl git \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

# The base image is multi-arch, so we have to install the mise build that matches the
# architecture of the image. The x64 build cannot run in an arm64 image.
ARG MISE_VERSION=v2026.4.28
ARG MISE_SHA256_X64=9655492db554e8f70a69830f54307ac0f4681d6c42f9844e862528b7853d09d1
ARG MISE_SHA256_ARM64=4a8b5ed8711744111cc92b2ecaeb3ebb24785f24d60bd83c1ce32bc2a85b0cb9
RUN case "$(dpkg --print-architecture)" in \
        amd64) mise_arch=x64; mise_sha256="${MISE_SHA256_X64}" ;; \
        arm64) mise_arch=arm64; mise_sha256="${MISE_SHA256_ARM64}" ;; \
        *) echo "No mise build is configured for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac \
    && curl -fsSL "https://github.com/jdx/mise/releases/download/${MISE_VERSION}/mise-${MISE_VERSION}-linux-${mise_arch}" \
        -o /usr/local/bin/mise \
    && echo "${mise_sha256}  /usr/local/bin/mise" | sha256sum -c \
    && chmod +x /usr/local/bin/mise

RUN useradd -m -s /bin/bash dev \
    && mkdir -p /workspace \
    && chown dev:dev /workspace

USER dev

RUN echo 'eval "$(mise activate bash)"' >> /home/dev/.bashrc
