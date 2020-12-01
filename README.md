# helium-gateway

![check](https://github.com/helium/gateway-rs/workflows/check/badge.svg)
![release](https://github.com/helium/gateway-rs/workflows/release/badge.svg)

helium-gateway is a gateway service between a linux based LoRa gateways using the a GWMP1/2 based packet forwarder, and the Helium router. 

The current gateway project forwards packets to the router but does **not** yet use state channels which means forwarded packets are not yet rewarded by the blockchain. 

The project builds `ipk` [packaged releases](https://github.com/helium/gateway-rs/releases) for linux based LoRa gateways. These packages attempt to be self-updating to be able to track improvements to the service. Updates are delivered through the following _channels_ which a gateway can subscribe to by a `channel` setting in the `update` section of the settings file:

* **alpha** - Early development releases. These will happen frequently as functionality is developed and may be unstable. Expect to need to log into your gateway to restart or manually fix your light gateway.
* **beta** - Pre-release candidates which are considered to be stable enough for early access. Breaking issues can still happen but should be rare. 
* **release** - The main (and default) release channel. Updates are considered to be stable for all platforms.


**NOTE**: Gateways should have at least **16Mb** of available application file space to handle gateway installation and updates.

## Installing

If your [supported LoRa gateway](#supported-platforms) did not come with helium-gateway pre-installed, manual installation requires you to:

1. Configure the packet forwarder on the gateway to forward to the helium-gateway application. This varies per gateway but the goal is to set the packet forwarder to forward to the (default) configured helium-gateway on `127.0.0.1` at udp port `1680`
2. Set up ssh acccess to the gateway. Depending on the gateway that may require going through a web interface, while others already have ssh configured. 
3. `scp` a downloaded `ipk` release package for the supported platform to the gateway. e.g. 
   ```shell
   scp helium-gateway-<version>-<platform>.ipk <gateway>:/tmp/</code>
   ```
4. `ssh` into the device and install the service using a command like:
   ```shell
   opkg install /tmp/helium-gateway-<version>-<platform>.ipk
   ```

If this command succeeds the logs on the gateway will show the service starting and the local packet forwarder client connecting to the gateway service. 

## Supported Platforms

| Platform       | Target                        | Products                                                   |
| -------------- | ----------------------------- | ---------------------------------------------------------- |
| [ramips_24kec] | mipsel-unknown-linux-musl     | * :white_check_mark: [RAK833] EVB Kit                      |
|                |                               | * :question: [RAK7258] (WisGate Edge Lite)                 |
|                |                               | * :question: [RAK7249] (WisGate Edge Max)                  |
|                |                               | * :question: [RAK7240] (WisGate Edge Prime)                |
| klkgw          | armv7-unknown-linux-musleabih | * :white_check_mark: Kerlink [Wirnet iFemtoCell Evolution] |


[ramips_24kec]: https://downloads.rakwireless.com/WIFI/RAK634/Hardware%20Specification/RAK634_Module_Specification_V1.0.pdf
[RAK833]: https://github.com/RAKWireless/RAK2247-RAK833-LoRaGateway-OpenWRT-MT7628
[RAK7258]: https://store.rakwireless.com/products/rak7258-micro-gateway
[RAK7249]: https://store.rakwireless.com/products/rak7249-diy-outdoor-gateway
[RAK7240]: https://store.rakwireless.com/products/rak7240-outdoor-lpwan-gateway?variant=36068284465310
[Wirnet iFemtoCell Evolution] https://www.kerlink.com/product/wirnet-ifemtocell-evolution/

## Building

Use one of the existing [releases](https://github.com/helium/gateway-rs/releases) if you can, or build your own by hand using the instructions below.

If you want to support a new platform, please consider submitting a PR to get the package as part of the supported matrix of gateway platforms!

1. Install `rust`
    ```shell
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    ```
2. Install cargo `cross` and `make`. The `cross` command allows for cross compiling to hardware targets using docker images, while the `make` command is used to package up 
   ```shell
   cargo install cross
   cargo install make
   ```
3. Build the application or package using one of the following:
   1. Build the application binary using the target triplet from the supported targets. Note the use of the `--release` flag to optimize the target binary for size. Debug builds may be too large to run on some targets. 
        ```shell
        cross build --target <target> --release
        ```
        The resulting application binary is located in
        ```
        target/<target>/release/helium_gateway
        ```

    2. Build an application `ipk` package using one of the target system profile names
        ```shell
        cargo make --profile <platform> ipk
        ```
        The resulting `ipk` will be located in
         ```
         target/ipk/helium-gateway-<version>-<platform>.ipk
         ```
    

