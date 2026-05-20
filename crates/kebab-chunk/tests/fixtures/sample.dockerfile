FROM rust:1.94-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release
CMD ["/app/target/release/kebab"]
