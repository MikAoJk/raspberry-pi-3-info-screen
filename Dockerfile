FROM rust:alpine@sha256:ec9c91e77119ce498cd1e87d96d77e0f75b2cee21655a29bc2bf75a51a2b20a4 AS builder
RUN cargo --version && cargo install cargo-auditable
WORKDIR /build
COPY . .
RUN cargo auditable build --release

FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3
WORKDIR /app
COPY --from=builder /build/target/release/infoscreeen /app/infoscreeen
COPY --from=builder /build/static /app/static
EXPOSE 8080
CMD ["/app/infoscreeen"]
