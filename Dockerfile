FROM rust:alpine AS builder
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
