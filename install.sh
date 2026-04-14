#!/bin/sh
# Arai installer — https://arai.taniwha.ai
# Usage: curl -sSf https://arai.taniwha.ai/install | sh
#
# Options (via environment variables):
#   ARAI_FULL=1         Install the full binary (with enrichment, ~32MB)
#   ARAI_VERSION=v0.1.0 Install a specific version
#   ARAI_INSTALL_DIR=   Override install directory (default: ~/.local/bin)

set -e

REPO="taniwhaai/arai"
BINARY_NAME="arai"

main() {
    detect_platform
    get_version
    select_binary
    download_binary
    install_binary
    print_success
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)  PLATFORM_OS="linux" ;;
        Darwin) PLATFORM_OS="darwin" ;;
        MINGW*|MSYS*|CYGWIN*)
            PLATFORM_OS="windows"
            ;;
        *)
            echo "Error: Unsupported operating system: $OS"
            exit 1
            ;;
    esac

    case "$ARCH" in
        x86_64|amd64)   PLATFORM_ARCH="x86_64" ;;
        aarch64|arm64)  PLATFORM_ARCH="aarch64" ;;
        *)
            echo "Error: Unsupported architecture: $ARCH"
            exit 1
            ;;
    esac

    PLATFORM="${PLATFORM_OS}-${PLATFORM_ARCH}"
    echo "  Detected platform: ${PLATFORM}"
}

get_version() {
    if [ -n "$ARAI_VERSION" ]; then
        VERSION="$ARAI_VERSION"
    else
        echo "  Fetching latest version..."
        VERSION=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | head -1 \
            | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

        if [ -z "$VERSION" ]; then
            echo "Error: Could not determine latest version."
            echo "Try setting ARAI_VERSION=v0.1.0 explicitly."
            exit 1
        fi
    fi

    echo "  Version: ${VERSION}"
}

select_binary() {
    if [ "${ARAI_FULL:-0}" = "1" ]; then
        VARIANT="arai-full"
        echo "  Variant: full (with enrichment)"
    else
        VARIANT="arai"
        echo "  Variant: lean"
    fi

    if [ "$PLATFORM_OS" = "windows" ]; then
        FILENAME="${VARIANT}-${PLATFORM}.exe"
    else
        FILENAME="${VARIANT}-${PLATFORM}"
    fi

    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${FILENAME}"
}

download_binary() {
    TMPDIR=$(mktemp -d)
    TMPFILE="${TMPDIR}/${BINARY_NAME}"

    echo "  Downloading ${FILENAME}..."
    HTTP_CODE=$(curl -sL -w "%{http_code}" -o "$TMPFILE" "$DOWNLOAD_URL")

    if [ "$HTTP_CODE" != "200" ]; then
        echo "Error: Download failed (HTTP ${HTTP_CODE})"
        echo "URL: ${DOWNLOAD_URL}"
        echo ""
        echo "Available at: https://github.com/${REPO}/releases"
        rm -rf "$TMPDIR"
        exit 1
    fi

    chmod +x "$TMPFILE"
}

install_binary() {
    if [ -n "$ARAI_INSTALL_DIR" ]; then
        INSTALL_DIR="$ARAI_INSTALL_DIR"
    elif [ "$(id -u)" = "0" ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        INSTALL_DIR="${HOME}/.local/bin"
    fi

    mkdir -p "$INSTALL_DIR"
    mv "$TMPFILE" "${INSTALL_DIR}/${BINARY_NAME}"
    rm -rf "$TMPDIR"

    INSTALLED_PATH="${INSTALL_DIR}/${BINARY_NAME}"
}

print_success() {
    echo ""
    echo "  ✓ Arai installed to ${INSTALLED_PATH}"
    echo ""

    # Check if install dir is in PATH
    case ":$PATH:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            echo "  ⚠ ${INSTALL_DIR} is not in your PATH."
            echo "  Add it with:"
            echo ""
            echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
            echo ""
            echo "  Or add that line to your ~/.bashrc or ~/.zshrc"
            echo ""
            ;;
    esac

    echo "  Get started:"
    echo ""
    echo "    cd your-project"
    echo "    arai init"
    echo ""
}

main
