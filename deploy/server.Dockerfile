# syntax=docker/dockerfile:1.7
#
# Image de production du serveur Rust LiveChat (`lc-server`, version workspace 3.0.0).
#
# Contrat : voir `docs/rust/packaging.md`.
# Construction (depuis la racine du dépôt) :
#   docker build -f deploy/server.Dockerfile -t livechat-server:3.0.0 .
#
# Choix de cache : pas de cargo-chef. Le workspace contient `apps/lc-overlay` (Tauri 2,
# build.rs + deps WebKit/GTK) : `cargo chef cook` travaille à l'échelle du workspace et
# tenterait de compiler ces dépendances, absentes d'une image slim. On utilise donc le
# pattern « manifests + sources factices » : les dépendances de `lc-server` sont
# compilées une fois dans une couche qui ne dépend que des manifestes/Cargo.lock ; le
# COPY des vraies sources ne recompile que les crates du workspace local.

ARG RUST_VERSION=1.99.1
ARG DEBIAN_RELEASE=bookworm

# ============================================================================
# Stage 1 — builder : dépendances (cache) puis binaire release `lc-server`
# ============================================================================
FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS builder

# `RUSTUP_TOOLCHAIN` neutralise `rust-toolchain.toml` (channel = "stable") pour
# garantir la reproductibilité et éviter un téléchargement de toolchain.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_BUILD_JOBS=4 \
    CARGO_TERM_COLOR=always \
    CARGO_INCREMENTAL=0 \
    RUST_BACKTRACE=1
WORKDIR /app

# --- 1a) Manifestes seuls : cette couche ne s'invalide que si un Cargo.toml ou
#         Cargo.lock change (pas à chaque modification de source).
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo/config.toml .cargo/config.toml
COPY crates/lc-protocol/Cargo.toml crates/lc-protocol/Cargo.toml
COPY crates/lc-domain/Cargo.toml crates/lc-domain/Cargo.toml
COPY crates/lc-db/Cargo.toml crates/lc-db/Cargo.toml
COPY crates/lc-media/Cargo.toml crates/lc-media/Cargo.toml
COPY crates/lc-playback/Cargo.toml crates/lc-playback/Cargo.toml
COPY crates/lc-discord/Cargo.toml crates/lc-discord/Cargo.toml
COPY crates/lc-admin/Cargo.toml crates/lc-admin/Cargo.toml
COPY apps/lc-server/Cargo.toml apps/lc-server/Cargo.toml
COPY apps/lc-overlay/Cargo.toml apps/lc-overlay/Cargo.toml
COPY xtask/Cargo.toml xtask/Cargo.toml

# --- 1b) Sources factices minimales : le workspace est valide, seules les
#         dépendances de `lc-server` sont compilées. Le workspace exige un
#         fichier source par cible (lib/main) de chaque membre.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    set -eu; \
    for dir in crates/*/; do \
        mkdir -p "${dir}src"; \
        : > "${dir}src/lib.rs"; \
    done; \
    for dir in apps/*/ xtask/; do \
        mkdir -p "${dir}src"; \
        : > "${dir}src/lib.rs"; \
        printf 'fn main() {}\n' > "${dir}src/main.rs"; \
    done; \
    printf 'fn main() {}\n' > apps/lc-overlay/build.rs; \
    cargo build --release --locked -p lc-server --bin lc-server

# --- 1c) Vraies sources : seules les crates du workspace local sont recompilées.
#         `COPY` conserve les mtimes de l'hôte ; les sources réelles pouvant être
#         plus anciennes que les artefacts du build factice, cargo les considérerait
#         à jour et le binaire final resterait celui des sources factices. On force
#         donc un mtime plus récent que la couche de build précédente.
COPY crates ./crates
COPY apps ./apps
COPY xtask ./xtask
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    find crates apps xtask -type f -exec touch {} + \
    && cargo build --release --locked -p lc-server --bin lc-server \
    && strip --strip-debug target/release/lc-server

# ============================================================================
# Stage 2 — runner : Debian slim, ffmpeg, yt-dlp épinglé, utilisateur non-root
# ============================================================================
FROM debian:${DEBIAN_RELEASE}-slim AS runtime

# yt-dlp : binaire standalone officiel `yt-dlp_linux`, version et SHA-256 épinglés
# (hash vérifié depuis `SHA2-256SUMS` de la release et le digest de l'asset GitHub).
# Mise à jour = bump des deux ARG ci-dessous, puis rebuild + smoke test.
ARG YTDLP_VERSION=2026.10.26
ARG YTDLP_SHA256=767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795

LABEL org.opencontainers.image.title="livechat-server" \
      org.opencontainers.image.description="Serveur LiveChat (Rust, protocole v2)" \
      org.opencontainers.image.version="3.0.0" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"

ENV DEBIAN_FRONTEND=noninteractive

# ffmpeg/ffprobe = sondage et transcodage médias ; curl = HEALTHCHECK ; tini = PID 1.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        ffmpeg \
        tini \
    && rm -rf /var/lib/apt/lists/*

RUN set -eu; \
    curl --fail --silent --show-error --location --retry 3 \
        --output /usr/local/bin/yt-dlp \
        "https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp_linux"; \
    echo "${YTDLP_SHA256}  /usr/local/bin/yt-dlp" | sha256sum --check --status; \
    chmod 0755 /usr/local/bin/yt-dlp; \
    /usr/local/bin/yt-dlp --version

# Utilisateur non-root dédié (uid/gid 10001) et répertoire de données.
RUN groupadd --gid 10001 livechat \
    && useradd --uid 10001 --gid 10001 --no-create-home \
        --home-dir /app --shell /bin/false livechat \
    && mkdir -p /app/data/media \
    && chown -R 10001:10001 /app

COPY --from=builder --chown=10001:10001 /app/target/release/lc-server /usr/local/bin/lc-server

# Défauts du conteneur (tous surchargeables par `docker run -e` / orchestration).
#  - API_URL est OBLIGATOIRE pour `lc-server` (origine CORS + base des URL médias) ;
#    à surcharger avec l'origine publique réelle en production.
#  - DATABASE_URL : format `sqlite:///chemin/absolu` (sqlx) ; le volume /app/data
#    conserve la base SQLite avec WAL.
#  - ADMIN_UI_LOCAL_ONLY=false : dans un conteneur, l'admin doit être joignable hors
#    loopback ; la protection repose alors sur ADMIN_UI_TOKEN (à fournir au runtime).
ENV PORT=3000 \
    API_URL=http://localhost:3000 \
    DATABASE_URL=sqlite:///app/data/livechat.sqlite \
    MEDIA_STORAGE_DIR=/app/data/media \
    RUST_LOG=info \
    ADMIN_UI_LOCAL_ONLY=false \
    MIN_OVERLAY_VERSION=3.0.0

WORKDIR /app
VOLUME ["/app/data"]
EXPOSE 3000
USER 10001:10001
STOPSIGNAL SIGTERM

# `/health` répond 200 même dégradé (contrat Phase 3a) : le HEALTHCHECK Docker ne
# distingue donc pas `degraded` ; les sondes métier lisent le JSON.
HEALTHCHECK --interval=15s --timeout=3s --start-period=15s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 2 "http://127.0.0.1:${PORT}/health" > /dev/null || exit 1

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/usr/local/bin/lc-server"]
