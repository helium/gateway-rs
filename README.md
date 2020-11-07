# gateway-rs


## Cross-compiling for OpenWRT 15.05, targeting [RAK634] (aka MT7628N) hardware

1. Install cargo `cross`

    ```shell
    ; cargo install cross
    ```

1. Build

    ```shell
    ; cross build --release --target mipsel-unknown-linux-musl
    ```

1. Copy `gateway` to the target machine

    ```
    ; scp target/mipsel-unknown-linux-musl/release/gateway user@target-machine:.
    ```

[RAK634]: https://downloads.rakwireless.com/WIFI/RAK634/Hardware%20Specification/RAK634_Module_Specification_V1.0.pdf
