#!/bin/bash

# Configure act to work with Podman
# Based on: https://github.com/nektos/act/issues/303 and related issues

echo "Starting Podman socket for act compatibility..."
systemctl --user enable --now podman.socket

echo "Setting up environment for act with Podman..."
export DOCKER_HOST=unix://$XDG_RUNTIME_DIR/podman/podman.sock

echo "Running act with Podman configuration..."
act --container-daemon-socket $XDG_RUNTIME_DIR/podman/podman.sock "$@"