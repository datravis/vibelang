#!/usr/bin/env bash
set -euo pipefail

# ── Benchmark Dependency Installer ──
# Installs everything needed to run bench.sh

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

ok()   { echo -e "  ${GREEN}[ok]${NC}   $1"; }
warn() { echo -e "  ${YELLOW}[skip]${NC} $1"; }
fail() { echo -e "  ${RED}[fail]${NC} $1"; }

echo -e "${BOLD}Checking benchmark dependencies...${NC}"
echo ""

MISSING=0

# ── Rust compiler ──
if command -v rustc &>/dev/null; then
    ok "rustc $(rustc --version | awk '{print $2}')"
else
    fail "rustc not found"
    echo "       Install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    MISSING=1
fi

# ── Go compiler ──
if command -v go &>/dev/null; then
    ok "go $(go version | awk '{print $3}' | sed 's/go//')"
else
    fail "go not found"
    echo "       Install: https://go.dev/dl/"
    MISSING=1
fi

# ── Python 3 ──
if command -v python3 &>/dev/null; then
    ok "python3 $(python3 --version | awk '{print $2}')"
else
    fail "python3 not found"
    echo "       Install: sudo apt-get install python3"
    MISSING=1
fi

# ── C compiler (cc) ──
if command -v cc &>/dev/null; then
    ok "cc ($(cc --version | head -1))"
else
    fail "cc (C compiler/linker) not found"
    echo "       Install: sudo apt-get install build-essential"
    MISSING=1
fi

# ── GNU time ──
if [ -x /usr/bin/time ]; then
    ok "/usr/bin/time (GNU time)"
else
    fail "/usr/bin/time not found (needed for memory/CPU measurement)"
    echo "       Install: sudo apt-get install time"
    MISSING=1
fi

# ── LLVM 18 ──
if [ -d /usr/lib/llvm-18 ]; then
    ok "LLVM 18 (/usr/lib/llvm-18)"
else
    warn "LLVM 18 not found at /usr/lib/llvm-18 (only needed to rebuild the compiler)"
fi

# ── VibeLang compiler ──
VIBE="$ROOT_DIR/compiler/target/debug/vibe"
VIBE_RELEASE="$ROOT_DIR/compiler/target/release/vibe"
if [ -x "$VIBE_RELEASE" ]; then
    ok "vibe compiler (release build)"
elif [ -x "$VIBE" ]; then
    ok "vibe compiler (debug build)"
    warn "consider building release for accurate benchmarks:"
    echo "       LLVM_SYS_180_PREFIX=/usr/lib/llvm-18 cargo build --manifest-path compiler/Cargo.toml --release"
else
    fail "vibe compiler not built"
    echo "       Build: LLVM_SYS_180_PREFIX=/usr/lib/llvm-18 cargo build --manifest-path compiler/Cargo.toml"
    MISSING=1
fi

echo ""

# ── Auto-install missing packages (Debian/Ubuntu) ──
if [ "$MISSING" -eq 1 ] && command -v apt-get &>/dev/null; then
    echo -e "${BOLD}Attempting to install missing dependencies...${NC}"
    echo ""

    PKGS=()
    command -v python3 &>/dev/null || PKGS+=(python3)
    command -v cc &>/dev/null      || PKGS+=(build-essential)
    [ -x /usr/bin/time ]           || PKGS+=(time)

    if [ ${#PKGS[@]} -gt 0 ]; then
        echo "  Installing: ${PKGS[*]}"
        sudo apt-get update -qq && sudo apt-get install -y -qq "${PKGS[@]}"
        echo ""
    fi

    # Rust
    if ! command -v rustc &>/dev/null; then
        echo "  Installing Rust via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env" 2>/dev/null || true
        echo ""
    fi

    # Go
    if ! command -v go &>/dev/null; then
        echo "  Installing Go..."
        GO_VERSION="1.22.2"
        curl -sL "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" | sudo tar -C /usr/local -xzf -
        export PATH=$PATH:/usr/local/go/bin
        echo ""
    fi

    # Build vibe compiler if not present
    if [ ! -x "$VIBE" ] && [ ! -x "$VIBE_RELEASE" ]; then
        if [ -d /usr/lib/llvm-18 ]; then
            echo "  Building VibeLang compiler..."
            LLVM_SYS_180_PREFIX=/usr/lib/llvm-18 cargo build --manifest-path "$ROOT_DIR/compiler/Cargo.toml"
            echo ""
        fi
    fi
fi

# ── Final check ──
echo -e "${BOLD}Verifying setup...${NC}"
echo ""

ALL_OK=true
for cmd in rustc go python3 cc; do
    if ! command -v "$cmd" &>/dev/null; then
        fail "$cmd still missing"
        ALL_OK=false
    fi
done
[ -x /usr/bin/time ] || { fail "/usr/bin/time still missing"; ALL_OK=false; }
[ -x "$VIBE" ] || [ -x "$VIBE_RELEASE" ] || { fail "vibe compiler not found"; ALL_OK=false; }

if $ALL_OK; then
    echo -e "  ${GREEN}${BOLD}All dependencies ready. Run:${NC}"
    echo ""
    echo "    ./benchmarks/bench.sh"
    echo ""
else
    echo ""
    echo -e "  ${RED}${BOLD}Some dependencies are still missing. See above for install instructions.${NC}"
    exit 1
fi
