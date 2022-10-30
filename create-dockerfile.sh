#!/bin/sh

cargo install dockerfile
cargo dockerfile --cmd /app/hass-grafana-bridge -u bridge -r debian:stable-slim
