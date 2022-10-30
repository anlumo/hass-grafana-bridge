FROM rust:latest as builder
RUN USER=root cargo new --bin hass-grafana-bridge

WORKDIR /hass-grafana-bridge

COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release
RUN rm src/*.rs ./target/release/deps/hass_grafana_bridge*
ADD . ./
RUN cargo build --release

FROM debian:stable-slim
ARG APP=/app
ARG APP_USER=bridge

RUN groupadd $APP_USER && useradd -g $APP_USER $APP_USER && mkdir -p $APP
COPY --from=builder /hass-grafana-bridge//target/release/hass-grafana-bridge $APP/hass-grafana-bridge

USER $USER
WORKDIR $APP
    
CMD ["/app/hass-grafana-bridge"]
