#!/bin/bash

# Configuration
INTERFACE_IN="veth0"
INTERFACE_OUT="veth1"
BPF_FS="/sys/fs/bpf"

echo "--- Configuring Network Environment for Constellation RPC Router ---"

# 1. Clean up existing interfaces if they exist
if ip link show "$INTERFACE_IN" > /dev/null 2>&1; then
    echo "[*] Removing existing $INTERFACE_IN..."
    sudo ip link del "$INTERFACE_IN"
fi

# 2. Create veth pair (virtual cable)
# veth0 will be used by the Router, veth1 will act as the Traffic Generator
echo "[*] Creating veth pair: $INTERFACE_IN <-> $INTERFACE_OUT"
sudo ip link add "$INTERFACE_IN" type veth peer name "$INTERFACE_OUT"

# 3. Bring interfaces UP
echo "[*] Setting interfaces to UP state..."
sudo ip link set "$INTERFACE_IN" up
sudo ip link set "$INTERFACE_OUT" up

# 4. Mount BPF File System (required for BPF Links and libxdp)
if ! mountpoint -q "$BPF_FS"; then
    echo "[*] Mounting BPF filesystem at $BPF_FS..."
    sudo mkdir -p "$BPF_FS"
    sudo mount -t bpf bpf "$BPF_FS"
else
    echo "[+] BPF filesystem is already mounted."
fi

# 5. Disable checksum offloading
# Useful for XDP to ensure packets are processed correctly in Generic mode
echo "[*] Disabling checksum offloading on $INTERFACE_IN..."
sudo ethtool -K "$INTERFACE_IN" tx off rx off > /dev/null 2>&1

echo "--- Setup Complete ---"
echo "Run your router with: sudo -E RUST_LOG=info cargo run -- --iface $INTERFACE_IN"
echo "Send test traffic to: $INTERFACE_OUT"
