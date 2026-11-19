FROM rust:1.100.0-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS native-source
WORKDIR /src/extore/plugins/commerce
# packages/extore is an exact gitlink; CI verifies it against the packaged pin.
COPY packages/extore/plugins/commerce/ ./
COPY packages/extore/LICENSE /src/extore/LICENSE

FROM native-source AS native-build
RUN cargo build --locked --release --jobs 2 --workspace
RUN target="$(rustc -vV | sed -n 's/^host: //p')" \
    && cargo metadata --locked --format-version 1 --filter-platform "$target" > /tmp/extore-metadata.json \
    && python3 scripts/collect_notices.py --metadata /tmp/extore-metadata.json --output-dir /licenses/extore

FROM rust:1.100.0-alpine3.22@sha256:f8ddfd4b265d9a9e7fd7c36821af4a55771aee2ac3894294687a66c709319be9 AS native-musl-build
ARG TARGETARCH
WORKDIR /src/extore/plugins/commerce
COPY packages/extore/plugins/commerce/ ./
COPY packages/extore/LICENSE /src/extore/LICENSE
RUN case "$TARGETARCH" in amd64) target=x86_64-unknown-linux-musl ;; arm64) target=aarch64-unknown-linux-musl ;; *) exit 1 ;; esac \
    && RUSTFLAGS="-C target-feature=-crt-static -C link-self-contained=no -C linker=cc" \
       cargo build --locked --release --jobs 2 --workspace --target "$target" --target-dir /native-target \
    && cargo metadata --locked --format-version 1 --filter-platform "$target" > /native-metadata.json \
    && mkdir -p /native-libraries \
    && cp "/native-target/$target/release/libextore_plugin_host.so" "/native-target/$target/release/libextore_commerce_plugin.so" /native-libraries/

# Collect notices from the actual musl Cargo graph in the fixed Python-capable
# GNU compiler image. The native musl build does not need an extra interpreter.
FROM native-source AS native-musl-notices
COPY --from=native-musl-build /usr/local/cargo/registry/ /usr/local/cargo/registry/
COPY --from=native-musl-build /native-metadata.json /native-metadata.json
RUN python3 scripts/collect_notices.py --metadata /native-metadata.json --output-dir /licenses/extore

FROM golang:1.27.2-bookworm@sha256:5cf287a799e6b94384bad13d16b14904c531f51ba65792237e122ce42b392f61 AS build
ARG TARGETARCH
WORKDIR /src
COPY services/extensions/ ./
RUN CGO_ENABLED=0 go build -mod=readonly -trimpath -ldflags="-s -w" -o /lmm-extensions ./cmd/extensions \
    && cp "$(go env GOMODCACHE)/github.com/ebitengine/purego@v0.11.3/LICENSE" /purego-LICENSE
RUN case "$TARGETARCH" in amd64) musl_arch=x86_64 ;; arm64) musl_arch=aarch64 ;; *) exit 1 ;; esac \
    && CGO_ENABLED=0 go build -mod=readonly -trimpath -ldflags="-s -w -I /lib/ld-musl-$musl_arch.so.1" -o /lmm-extensions-musl ./cmd/extensions

FROM scratch AS binaries
COPY --from=build /lmm-extensions /lmm-extensions
COPY --from=native-build /src/extore/plugins/commerce/target/release/libextore_plugin_host.so /lib/extore/libextore_plugin_host.so
COPY --from=native-build /src/extore/plugins/commerce/target/release/libextore_commerce_plugin.so /lib/extore/libextore_commerce_plugin.so
COPY --from=native-build /src/extore/LICENSE /licenses/extore/LICENSE
COPY --from=native-build /licenses/extore/ /licenses/extore/
COPY --from=build /purego-LICENSE /licenses/purego/LICENSE
COPY --from=build /usr/local/go/LICENSE /licenses/purego/GO-LICENSE

FROM scratch AS binaries-musl
COPY --from=build /lmm-extensions-musl /lmm-extensions
COPY --from=native-musl-build /native-libraries/ /lib/extore/
COPY --from=native-musl-build /src/extore/LICENSE /licenses/extore/LICENSE
COPY --from=native-musl-notices /licenses/extore/ /licenses/extore/
COPY --from=build /purego-LICENSE /licenses/purego/LICENSE
COPY --from=build /usr/local/go/LICENSE /licenses/purego/GO-LICENSE

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
COPY --from=native-build /src/extore/plugins/commerce/target/release/libextore_plugin_host.so /usr/lib/lmm/extensions/extore/libextore_plugin_host.so
COPY --from=native-build /src/extore/plugins/commerce/target/release/libextore_commerce_plugin.so /usr/lib/lmm/extensions/extore/libextore_commerce_plugin.so
COPY --from=native-build /src/extore/LICENSE /usr/share/licenses/lmm-extensions/extore/LICENSE
COPY --from=native-build /licenses/extore/ /usr/share/licenses/lmm-extensions/extore/
COPY --from=build /purego-LICENSE /usr/share/licenses/lmm-extensions/purego/LICENSE
COPY --from=build /usr/local/go/LICENSE /usr/share/licenses/lmm-extensions/purego/GO-LICENSE
COPY packaging/extore-plugin.json /usr/share/lmm-extensions/packaging/extore-plugin.json
COPY LICENSE NOTICE THIRD-PARTY-LICENSES.md /usr/share/licenses/lmm/
COPY config/legal/site.example.toml /usr/share/lmm/legal/site.example.toml
COPY config/legal/templates/ /usr/share/lmm/legal/templates/
USER 65532:65532
EXPOSE 8081
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent --max-time 2 http://127.0.0.1:8081/health/live || exit 1
ENTRYPOINT ["/usr/bin/lmm-extensions"]
CMD ["--config", "/etc/lmm.conf.d/extensions.toml"]
