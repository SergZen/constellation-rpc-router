# constellation-rpc-router

High-performance RPC transaction router for the **Solana Constellation** protocol.

### Overview

This project implements a low-latency transaction dispatcher designed for concurrent block production. It leverages **eBPF (XDP)** for kernel-level filtering and **AF\_XDP** for zero-copy data transfer to userspace.

**Key Features:**

*   **Kernel Bypass:** Bypasses the standard Linux networking stack using AF\_XDP for microsecond-level latency.
*   **Constellation Logic:** Parses 16-bit proposer bitmaps from incoming transaction packets to perform fan-out routing.
*   **Dynamic Control Plane:** Implements thread-safe (Arc/RwLock) proposer rotation, simulating real-time leader schedule updates.
*   **Zero-Copy Architecture:** Minimal CPU overhead by sharing memory (UMEM) between the NIC and the application.

## Prerequisites

1. stable rust toolchains: `rustup toolchain install stable`
2. nightly rust toolchains: `rustup toolchain install nightly --component rust-src`
3. (if cross-compiling) rustup target: `rustup target add ${ARCH}-unknown-linux-musl`
4. (if cross-compiling) LLVM: (e.g.) `brew install llvm` (on macOS)
5. bpf-linker: `cargo install bpf-linker` (`--no-default-features` on macOS)
6.  **System Dependencies:** `sudo apt install -y build-essential m4 pkg-config libelf-dev clang llvm gcc-multilib libpcap-dev`
7.  **Network Tools:** `sudo apt install -y iproute2 ethtool tcpdump python3-scapy` (for testing and debugging).

### Environment Setup

Since XDP requires specific network configurations, use the provided setup script to create a virtual testing environment:

```shell
chmod +x ./scripts/setup_env.sh
./scripts/setup_env.sh
```

This script:

*   Creates a `veth0 <-> veth1` virtual pair.
*   Mounts the BPF filesystem (`bpffs`) at `/sys/fs/bpf`.
*   Disables hardware offloading on virtual interfaces to ensure Compatibility with XDP Generic (Skb) mode.

## Build & Run

Use `cargo build`, `cargo check`, etc. as normal. Run your program with:

### Running the Router

Run the program with root privileges (required for BPF and AF\_XDP):


```shell
sudo -E RUST_LOG=info cargo run --release -- --iface veth0
```    
Cargo build scripts are used to automatically build the eBPF correctly and include it in the
program.

### Testing the Dispatcher

In a separate terminal, use the provided Python script to generate transactions with specific proposer bitmaps:

```shell
sudo python3 ./scripts/test_sender.py
```

**Note:** The router expects UDP packets on port 8000 with a 2-byte bitmap at the start of the payload. The simulation will log routing decisions to 16 virtual proposers.

### Verifying Traffic (Network Observation)

To see the real-time fan-out in action, open a third terminal and monitor the `veth1` interface. You will see the original packet entering the system and multiple redirected packets exiting it:

```shell
sudo tcpdump -i veth1 -n -vv udp
```    

**What to look for:**

1.  **Inbound:** A packet from `1.1.1.1` to `1.1.1.1` on port `8000`.
2.  **Outbound (Fan-out):** Multiple packets with the same payload but redirected to different ports (e.g., `9000-10000`) based on the active proposer rotation.

> **Note:** We monitor `veth1` because in our virtual setup, packets sent by the router from `veth0` physically arrive at the other end of the pipe (`veth1`).

## Cross-compiling on macOS

Cross compilation should work on both Intel and Apple Silicon Macs.

```shell
cargo build --package constellation-rpc-router --release \
  --target=${ARCH}-unknown-linux-musl \
  --config=target.${ARCH}-unknown-linux-musl.linker=\"rust-lld\"
```
The cross-compiled program `target/${ARCH}-unknown-linux-musl/release/constellation-rpc-router` can be
copied to a Linux server or VM and run there.

## License

With the exception of eBPF code, constellation-rpc-router is distributed under the terms
of either the [MIT license] or the [Apache License] (version 2.0), at your
option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.

### eBPF

All eBPF code is distributed under either the terms of the
[GNU General Public License, Version 2] or the [MIT license], at your
option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the GPL-2 license, shall be
dual licensed as above, without any additional terms or conditions.

[Apache license]: LICENSE-APACHE
[MIT license]: LICENSE-MIT
[GNU General Public License, Version 2]: LICENSE-GPL2
