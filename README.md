# gateway-rs


## Cross-compiling for OpenWRT 15.05, targeting [RAK634] (aka MT7628N) hardware

1. Download OpenWRT [toolchain]. You are correct; this toolchain is for a much newer version of OpenWRT than 15.05.
1. Unpack the toolchain and place in your path

    ```
    ; tar -xf lede-sdk-17.01.0-ramips-mt7628_gcc-5.4.0_musl-1.1.16.Linux-x86_64.tar.xz
    ; export PATH="path/to/lede-sdk-17.01.0-ramips-mt7628_gcc-5.4.0_musl-1.1.16.Linux-x86_64/staging_dir/toolchain-mipsel_24kc_gcc-5.4.0_musl-1.1.16/bin"
    ```

1. Copy _dynamic_ musl libc to the target system

    ```
    ; scp path/to/lede-sdk-17.01.0-ramips-mt7628_gcc-5.4.0_musl-1.1.16.Linux-x86_64/staging_dir/toolchain-mipsel_24kc_gcc-5.4.0_musl-1.1.16/lib/ld-musl-mipsel-sf.so.1 user@target-machine:/lib/
    ```

1. Install Rust little-endian musl target:

    ```
    ; rustup target add mipsel-unknown-linux-musl
    ```

1. Compile `gateway`

    ```
    ; cargo build --target mipsel-unknown-linux-musl
    ```

1. Copy `gateway` to the target machine

    ```
    ; scp target/mipsel-unknown-linux-musl/release/gateway user@target-machine:.
    ```

1. Log into the target machine and run

    ```
    ; ssh user@target-machine
    ; ./gateway -h
    ; gateway
    ```

[toolchain]: https://archive.openwrt.org/releases/17.01.0/targets/ramips/mt7628/lede-sdk-17.01.0-ramips-mt7628_gcc-5.4.0_musl-1.1.16.Linux-x86_64.tar.xz
[RAK634]: https://downloads.rakwireless.com/WIFI/RAK634/Hardware%20Specification/RAK634_Module_Specification_V1.0.pdf
