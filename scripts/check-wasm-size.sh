#!/usr/bin/env bash
# Check WASM size for StreamPay contract
# Reports optimized size and compares against Soroban limits
#
# Requirements: stellar-cli (install with: cargo install stellar-cli)

set -e

# Colors for output
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Soroban limits
MAX_SIZE=262144      # 256 KB hard limit
WARN_SIZE=204800     # 200 KB warning threshold (78% of limit)

echo -e "${BLUE}=== StreamPay WASM Size Check ===${NC}\n"

# Check if stellar CLI is available
if ! command -v stellar &> /dev/null; then
    echo -e "${YELLOW}Warning: stellar CLI not found${NC}"
    echo "Install with: cargo install stellar-cli"
    echo ""
    echo "Checking for existing WASM build..."
    WASM_FILE="target/wasm32-unknown-unknown/release/streampay_contracts.wasm"
    
    if [ ! -f "$WASM_FILE" ]; then
        echo -e "${RED}Error: No WASM file found and stellar CLI not available${NC}"
        echo "Please install stellar-cli to build the contract."
        exit 1
    fi
    echo "Using existing build (may not be current)"
else
    echo "Building optimized WASM with stellar CLI..."
    stellar contract build --release
    WASM_FILE="target/wasm32-unknown-unknown/release/streampay_contracts.wasm"
fi

if [ ! -f "$WASM_FILE" ]; then
    echo -e "${RED}Error: WASM file not found at $WASM_FILE${NC}"
    exit 1
fi

# Get file size
SIZE=$(wc -c < "$WASM_FILE")
SIZE_KB=$((SIZE / 1024))
PERCENT=$((SIZE * 100 / MAX_SIZE))

echo -e "\n${BLUE}Results:${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "WASM file:     $(basename $WASM_FILE)"
echo -e "Size:          ${GREEN}${SIZE}${NC} bytes (${SIZE_KB} KB)"
echo -e "Limit:         ${MAX_SIZE} bytes (256 KB)"
echo -e "Usage:         ${PERCENT}%"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check against thresholds
if [ "$SIZE" -gt "$MAX_SIZE" ]; then
    echo -e "\n${RED}❌ ERROR: WASM size exceeds Soroban limit!${NC}"
    echo -e "   Size: ${SIZE} bytes > ${MAX_SIZE} bytes"
    echo -e "   Contract cannot be deployed to Soroban."
    exit 1
elif [ "$SIZE" -gt "$WARN_SIZE" ]; then
    echo -e "\n${YELLOW}⚠️  WARNING: WASM size approaching limit${NC}"
    echo -e "   Size: ${SIZE} bytes (${PERCENT}% of limit)"
    echo -e "   Consider optimizing to leave headroom for future features."
    exit 0
else
    echo -e "\n${GREEN}✓ WASM size is within acceptable limits${NC}"
    echo -e "  Headroom: $((MAX_SIZE - SIZE)) bytes ($((100 - PERCENT))% remaining)"
fi

# Optional: Show size breakdown if wasm-opt is available
if command -v wasm-opt &> /dev/null; then
    echo -e "\n${BLUE}Optimization potential:${NC}"
    TEMP_FILE=$(mktemp)
    wasm-opt -Oz "$WASM_FILE" -o "$TEMP_FILE" 2>/dev/null || true
    if [ -f "$TEMP_FILE" ]; then
        OPT_SIZE=$(wc -c < "$TEMP_FILE")
        OPT_SIZE_KB=$((OPT_SIZE / 1024))
        SAVINGS=$((SIZE - OPT_SIZE))
        if [ "$SAVINGS" -gt 0 ]; then
            SAVINGS_PERCENT=$((SAVINGS * 100 / SIZE))
            echo -e "  With wasm-opt -Oz: ${OPT_SIZE} bytes (${OPT_SIZE_KB} KB)"
            echo -e "  Potential savings: ${SAVINGS} bytes (${SAVINGS_PERCENT}%)"
        else
            echo -e "  Already optimally compressed"
        fi
        rm "$TEMP_FILE"
    fi
fi

echo ""
