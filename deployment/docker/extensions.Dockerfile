FROM golang:1.27.2-bookworm@sha256:5cf287a799e6b94384bad13d16b14904c531f51ba65792237e122ce42b392f61 AS build
WORKDIR /src
COPY services/extensions/ ./
RUN CGO_ENABLED=0 go build -mod=readonly -trimpath -ldflags="-s -w" -o /lmm-extensions ./cmd/extensions

FROM scratch AS binaries
COPY --from=build /lmm-extensions /lmm-extensions

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS runtime
# Keep runtime packages and their dependencies on the immutable snapshot from
# which this digest-pinned Debian base was created.
RUN set -eux; \
    sed -i \
      -e 's#http://deb.debian.org/debian-security#http://snapshot.debian.org/archive/debian-security/20261005T000000Z#' \
      -e 's#http://deb.debian.org/debian#http://snapshot.debian.org/archive/debian/20261005T000000Z#' \
      /etc/apt/sources.list.d/debian.sources; \
    apt-get -o Acquire::Check-Valid-Until=false update; \
    apt-get install --no-install-recommends -y \
      ca-certificates=20250419~deb12u1 \
      curl=7.88.1-10+deb12u15; \
    rm -rf /var/lib/apt/lists/*
COPY --from=build /lmm-extensions /usr/bin/lmm-extensions
COPY LICENSE NOTICE THIRD-PARTY-LICENSES.md /usr/share/licenses/lmm/
COPY config/legal/site.example.toml /usr/share/lmm/legal/site.example.toml
COPY config/legal/templates/ /usr/share/lmm/legal/templates/
USER 65532:65532
EXPOSE 8081
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent --max-time 2 http://127.0.0.1:8081/health/live || exit 1
ENTRYPOINT ["/usr/bin/lmm-extensions"]
CMD ["--config", "/etc/lmm.conf.d/extensions.toml"]

# Optional native bundle. Default binaries/runtime targets above remain Go-only.
# BuildKit runs this stage on TARGETPLATFORM; cross-linking is not implicit.
FROM rust:1.100.0-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS extore-build
ARG TARGETARCH
ARG LMM_SOURCE_REVISION
ARG EXTORE_SOURCE_REVISION
ENV CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/build
WORKDIR /src
RUN set -eux; \
    sed -i \
      -e 's#http://deb.debian.org/debian-security#http://snapshot.debian.org/archive/debian-security/20261005T000000Z#' \
      -e 's#http://deb.debian.org/debian#http://snapshot.debian.org/archive/debian/20261005T000000Z#' \
      /etc/apt/sources.list.d/debian.sources; \
    apt-get -o Acquire::Check-Valid-Until=false update; \
    apt-get install --no-install-recommends -y python3 binutils pkg-config libssl-dev; \
    rm -rf /var/lib/apt/lists/*
COPY services/extensions/extore-host/ services/extensions/extore-host/
COPY third_party/extore/ third_party/extore/
COPY scripts/build-distribution.py scripts/build-distribution.py
COPY packaging/distribution.json packaging/distribution.json
RUN set -eux; \
    case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu ;; \
      *) echo "Extore bundle supports only amd64/arm64 Linux GNU" >&2; exit 1 ;; \
    esac; \
    test "$(rustc -vV | sed -n 's/^host: //p')" = "$target"; \
    pkg-config --exists openssl; \
    cargo build --locked --release --target "$target" \
      --manifest-path third_party/extore/Cargo.toml --package extore --package extore-zplugin; \
    cargo build --locked --release --target "$target" \
      --manifest-path services/extensions/extore-host/Cargo.toml; \
    mkdir /out; \
    install -m 0755 "/build/$target/release/lmm-extore-host" /out/lmm-extore-host; \
    install -m 0644 "/build/$target/release/libextore_zplugin.so" /out/libextore_zplugin.so; \
    install -m 0755 "/build/$target/release/extore" /out/extore; \
    python3 scripts/build-distribution.py record-extore --input /out \
      --revision "$LMM_SOURCE_REVISION" --extore-revision "$EXTORE_SOURCE_REVISION" --target "$target"

FROM binaries AS binaries-extore
COPY --from=extore-build /out/lmm-extore-host /lmm-extore-host
COPY --from=extore-build /out/libextore_zplugin.so /libextore_zplugin.so
COPY --from=extore-build /out/extore-plugin.json /extore-plugin.json

# Extore HTTP executable is a disposable test fixture, never part of the LMM archive.
FROM scratch AS extore-test-binaries
COPY --from=extore-build /out/extore /extore

FROM runtime AS runtime-extore
USER 0:0
COPY --from=extore-build /out/lmm-extore-host /usr/libexec/lmm-extore-host
COPY --from=extore-build /out/libextore_zplugin.so /usr/lib/lmm/libextore_zplugin.so
COPY --from=extore-build /out/extore-plugin.json /usr/share/lmm/extore-plugin.json
COPY third_party/extore/LICENSE /usr/share/licenses/lmm/extore/LICENSE
# Resolve both actual ELFs against this pinned runtime, including versioned
# libc/OpenSSL dependencies. A copied ELF header alone is not runtime evidence.
RUN set -eux; \
    case "$(dpkg --print-architecture)" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu ;; \
      *) exit 1 ;; \
    esac; \
    grep -Fq "\"target\": \"$target\"" /usr/share/lmm/extore-plugin.json; \
    dpkg-query -W libc6 libssl3 > /usr/share/lmm/extore-runtime-dependencies.txt; \
    for binary in /usr/libexec/lmm-extore-host /usr/lib/lmm/libextore_zplugin.so; do \
      ldd "$binary" > /tmp/extore-ldd.txt 2>&1; \
      ! grep -F 'not found' /tmp/extore-ldd.txt; \
      cat /tmp/extore-ldd.txt >> /usr/share/lmm/extore-runtime-dependencies.txt; \
    done; \
    rm /tmp/extore-ldd.txt
USER 65532:65532

# Preserve docker build / compose defaults; native loading still requires config.
FROM runtime AS default
