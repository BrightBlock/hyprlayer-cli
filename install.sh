#!/bin/bash
# HyprLayer Installer - Version 1.0.0
# Install script for hyprlayer CLI

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Installation directories
INSTALL_DIR="$HOME/.hyprlayer"
BIN_DIR="$INSTALL_DIR/bin"

# Get latest release info
REPO="BrightBlock/hyprlayer-cli"
GITHUB_API="https://api.github.com/repos/$REPO/releases/latest"
DOWNLOAD_URL_BASE="https://github.com/$REPO/releases/download/v1.0.0"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        case "$ARCH" in
            x86_64)
                BINARY="hyprlayer-x86_64-unknown-linux-gnu"
                ;;
            aarch64)
                BINARY="hyprlayer-aarch64-unknown-linux-gnu"
                ;;
            *)
                echo -e "${RED}Error: Unsupported architecture: $ARCH${NC}"
                echo "Please use cargo install for this architecture"
                exit 1
                ;;
        esac
        ;;
    Darwin)
        case "$ARCH" in
            x86_64)
                BINARY="hyprlayer-x86_64-apple-darwin"
                ;;
            arm64)
                BINARY="hyprlayer-aarch64-apple-darwin"
                ;;
            *)
                echo -e "${RED}Error: Unsupported architecture: $ARCH${NC}"
                echo "Please use cargo install for this architecture"
                exit 1
                ;;
        esac
        ;;
    *)
        echo -e "${RED}Error: Unsupported OS: $OS${NC}"
        echo "Please use cargo install for this OS"
        exit 1
        ;;
esac

echo -e "${GREEN}Installing HyprLayer v1.0.0...${NC}"

# Check for existing installation
if [ -d "$INSTALL_DIR" ]; then
    echo -e "${YELLOW}Warning: HyprLayer is already installed at $INSTALL_DIR${NC}"
    read -p "Do you want to reinstall? [y/N] " -n 1 -r
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Installation cancelled."
        exit 0
    fi
    rm -rf "$INSTALL_DIR"
fi

# Create installation directories
mkdir -p "$BIN_DIR"
mkdir -p "$INSTALL_DIR"

# Download binary
DOWNLOAD_URL="$DOWNLOAD_URL_BASE/$BINARY"
echo "Downloading from: $DOWNLOAD_URL"

if command -v curl &> /dev/null; then
    curl -L -o "$BIN_DIR/hyprlayer" "$DOWNLOAD_URL"
elif command -v wget &> /dev/null; then
    wget -O "$BIN_DIR/hyprlayer" "$DOWNLOAD_URL"
else
    echo -e "${RED}Error: Neither curl nor wget is installed${NC}"
    exit 1
fi

# Make binary executable
chmod +x "$BIN_DIR/hyprlayer"

# Install Claude Code agents and commands
CLAUDE_DEST="$HOME/.claude"
ARCHIVE_URL="https://github.com/$REPO/archive/refs/tags/v1.0.0.tar.gz"

echo "Installing Claude Code agents and commands..."
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

if command -v curl &> /dev/null; then
    curl -sL -o "$TMPDIR/repo.tar.gz" "$ARCHIVE_URL"
elif command -v wget &> /dev/null; then
    wget -qO "$TMPDIR/repo.tar.gz" "$ARCHIVE_URL"
fi

tar -xzf "$TMPDIR/repo.tar.gz" -C "$TMPDIR"
EXTRACTED_DIR=$(find "$TMPDIR" -maxdepth 1 -type d -name "hyprlayer-cli-*" | head -1)

if [ -d "$EXTRACTED_DIR/claude" ]; then
    mkdir -p "$CLAUDE_DEST"
    cp -r "$EXTRACTED_DIR/claude"/. "$CLAUDE_DEST"/
    echo -e "${GREEN}Claude Code configuration installed to $CLAUDE_DEST${NC}"
else
    echo -e "${YELLOW}Warning: Could not find Claude Code configuration in release archive${NC}"
fi

# Add to PATH
echo ""
echo -e "${GREEN}Installation successful!${NC}"
echo ""
echo "HyprLayer has been installed to: $BIN_DIR"
echo ""
echo -e "${YELLOW}To use hyprlayer, add it to your PATH:${NC}"
echo ""

# Detect shell and provide PATH instructions
SHELL_NAME=$(basename "$SHELL")

case "$SHELL_NAME" in
    bash)
        echo "Add this line to your ~/.bashrc:"
        echo -e "${GREEN}export PATH=\"\$PATH:$BIN_DIR\"${NC}"
        echo ""
        echo "Then run: source ~/.bashrc"
        ;;
    zsh)
        echo "Add this line to your ~/.zshrc:"
        echo -e "${GREEN}export PATH=\"\$PATH:$BIN_DIR\"${NC}"
        echo ""
        echo "Then run: source ~/.zshrc"
        ;;
    fish)
        echo "Add this line to your ~/.config/fish/config.fish:"
        echo -e "${GREEN}set -gx PATH \$PATH $BIN_DIR${NC}"
        echo ""
        echo "Then run: source ~/.config/fish/config.fish"
        ;;
    *)
        echo "Add $BIN_DIR to your PATH environment variable"
        ;;
esac

echo ""
echo -e "${GREEN}To verify installation, run:${NC}"
echo "  hyprlayer --version"
echo ""
echo -e "${YELLOW}To uninstall, simply remove:$INSTALL_DIR${NC}"
