# Helium Gateway

[![ci](https://github.com/helium/gateway-rs/workflows/ci/badge.svg)](https://github.com/helium/gateway-rs/actions)

The Helium Gateway application is a service designed to run on Linux-based LoRaWAN gateways.

It's intended to run alongside a typical LoRa packet forwarder and to connect via Semtech's Gateway Messaging Protocol (GWMP, using JSON v1 or v2).

In turn, the Helium Gateway application does two things:
 * fetches blockchain context, such as routing tables and OUI endpoints, from a `Gateway Service`; this means the application does not need to maintain a full ledger of copy of the blockchain
 * connects and routes packets to the appropriates OUI endpoints (ie: `Helium Routers`)

```
                                                                 +-----------+
+-----------+                       +------------+               |  Gateway  |
|           |                       |            |<--- gRPC ---->|  Service  |
|  packet   |<--- Semtech GWMP ---->|   Helium   |               +-----------+
| forwarder |       over UDP        |   Gateway  |               +-----------+
|           |                       |            |<--- gRPC ---->|  Helium   |
+-----------+                       +------------+               |  Routers  |
                                                                 +-----------+
```

The current gateway project forwards packets to the router but does **not** yet use state channels which means forwarded packets are not yet rewarded by the blockchain.

The project builds `ipk` [packaged releases](https://github.com/helium/gateway-rs/releases) for Linux-based LoRa gateways. These packages attempt to be self-updating to be able to track improvements to the service. Updates are delivered through the following _channels_ which a gateway can subscribe to by a `channel` setting in the `update` section of the settings file:

* **alpha** - Early development releases. These will happen frequently as functionality is developed and may be unstable. Expect to need to log into your gateway to restart or manually fix your light gateway.
* **beta** - Pre-release candidates which are considered to be stable enough for early access. Breaking issues can still happen but should be rare.
* **release** - The main release channel. Updates are considered to be stable for all platforms.
* **semver** - This is the default channel and selects the channel based on the installed package version identifier.

**NOTE**: Gateways should have at least **16Mb** of available application file space to handle gateway installation and updates.

## Linux Dependencies

This application requires a Linux-based environment for two big reasons:
* `tokio`: the `gateway-rs` application, written in Rust, depends on [Tokio](https://docs.rs/tokio) for its runtime. Tokio binds to Linux interfaces such as `epoll` and `kqeueue`. It is technically possible to port Tokio to another OS or RTOS (this has been done for Windows), but it would be no simple undertaking.
* `curl`: for fetching releases over SSL, `curl` is used. This was a simple way to use SSL without bloating the `helium_gateway` binary with additional libraries. Note that the updater may be disabled and thus this dependency may be removed.

## Installing

If your [supported LoRa gateway](#supported-platforms) did not come with helium-gateway pre-installed, manual installation requires you to:

1. Configure the packet forwarder on the gateway to forward to the helium-gateway application. This varies per gateway but the goal is to set the packet forwarder to forward to the (default) configured helium-gateway on `127.0.0.1` at udp port `1680`
2. Set up ssh acccess to the gateway. Depending on the gateway that may require going through a web interface, while others already have ssh configured.
3. `scp` a downloaded release package for the supported platform to the gateway. e.g.
   ```shell
   scp helium-gateway-<version>-<platform>.ipk <gateway>:/tmp/
   ```
4. `ssh` into the device and install the service using a command like:
   ```shell
   opkg install /tmp/helium-gateway-<version>-<platform>.ipk
   ```
   or
   ```shell
   dpkg --install /tmp/helium-gateway-<version>-<platform>.deb
   ```
   **NOTE**: Some platform have custom package installation requirements. Refer to the developer instructions for that platform on how to install a package.

   The default region of the gateway is `US915`, if your region is different you can set the right one in `/etc/helium_gateway/settings.toml`. Just add the following line :
   ```shell
   region = "<region>"
   ```
   Possible values are : `US915| EU868 | EU433 | CN470 | CN779 | AU915 | AS923_1 | AS923_2 | AS923_3 | AS923_4 | KR920 | IN865`. After updating the value you need to restart the service :
   ```shell
   /etc/init.d/helium_gateway restart
   ```

If this command succeeds the logs on the gateway will show the service starting and the local packet forwarder client connecting to the gateway service.

## Supported Platforms

The following platforms have already been tested by Helium and our community. Our plan is to test this on all relevant hardware platforms used by the Helium Network. If your preferred platform isn't listed yet, here's how to get it added.

* Review [the open issues](https://github.com/helium/gateway-rs/issues) to see if it's already in progress. If not, file an issue.
* Join the `#gateway` channel on [Helium Discord](https://discord.gg/helium) and let us know what platform we're missing.

Note that platforms will be tested much faster if you join the development process!


| Platform       | Target                         | Products                                                 |
| -------------- | ------------------------------ | -------------------------------------------------------- |
| [ramips_24kec] | mipsel-unknown-linux-musl      | :white_check_mark: [RAK833] EVB Kit                      |
|                |                                | :white_check_mark: [RAK7258] (WisGate Edge Lite)         |
|                |                                | :grey_question: [RAK7249] (WisGate Edge Max)             |
|                |                                | :grey_question: [RAK7240] (WisGate Edge Prime)           |
| klkgw          | armv7-unknown-linux-musleabihf | :white_check_mark: Kerlink [Wirnet iFemtoCell Evolution] |
| dragino        | mips-unknown-linux-musl        | :white_check_mark: Dragino [LPS8]                        |
|                |                                | :grey_question: Dragino [DLOS8]                          |
| mtcdt          | armv5te-unknown-linux-musleabi | :white_check_mark: Multitech Conduit [MTCDT] (mLinux)    |
| resiot         | armv7-unknown-linux-gnueabihf  | :white_check_mark: ResIOT X gateways [resiot]            |
| cotx           | aarch64-unknown-linux-gnu      | :white_check_mark: Cotx gateways [cotx]               |
| x86_64         | x86_64-unknown-linux-gnu       | :white_check_mark: Debian x86_64                         |
| raspi01        | arm-unknown-linux-gnueabihf    | :grey_question: Raspberry Pi 0 or 1 running Raspian / Raspberry Pi OS or another Debian-based Linux distro        |
| raspi234       | armv7-unknown-linux-gnueabihf  | :grey_question: Raspberry Pi 2, 3, or 4 running Raspian / Raspberry Pi OS or another Debian-based Linux distro    |
| raspi_64        | aarch64-unknown-linux-gnu      | :grey_question: Raspberry Pi 3 or 4 running Raspian / Raspberry Pi OS 64 bit or another 64 bit Debian-based Linux distro |

[ramips_24kec]: https://downloads.rakwireless.com/WIFI/RAK634/Hardware%20Specification/RAK634_Module_Specification_V1.0.pdf
[RAK833]: https://github.com/RAKWireless/RAK2247-RAK833-LoRaGateway-OpenWRT-MT7628
[RAK7258]: https://store.rakwireless.com/products/rak7258-micro-gateway
[RAK7249]: https://store.rakwireless.com/products/rak7249-diy-outdoor-gateway
[RAK7240]: https://store.rakwireless.com/products/rak7240-outdoor-lpwan-gateway?variant=36068284465310
[Wirnet iFemtoCell Evolution]: https://www.kerlink.com/product/wirnet-ifemtocell-evolution/
[LPS8]: https://www.dragino.com/products/lora-lorawan-gateway/item/148-lps8.html
[DLOS8]: https://www.dragino.com/products/lora-lorawan-gateway/item/160-dlos8.html
[MTCDT]: https://www.multitech.com/brands/multiconnect-conduit
[resiot]: https://www.resiot.io/en/resiot-gateways/
[cotx]: https://www.cotxnetworks.com/product/service_one

## Building

Use one of the existing [releases](https://github.com/helium/gateway-rs/releases) if you can, or build your own by hand using the instructions below.

If you want to support a new platform, please consider submitting a PR to get the package as part of the supported matrix of gateway platforms!

1. Install `rust`
    ```shell
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    ```
2. Install cargo `cross`, `cargo-make`, and, if needed, `cargo-deb`. The `cross` command allows for cross compiling to hardware targets using docker images, while the `cargo-make` command is used to package up. If creating a deb package, `cargo-deb` is also needed.
   ```shell
   cargo install cross
   cargo install cargo-make
   cargo install cargo-deb
   ```
3. Build the application or package using one of the following:
   1. Build the application binary using the target triplet from the supported targets. Note the use of the `--release` flag to optimize the target binary for size. Debug builds may be to large to run on some targets.
        ```shell
        cross build --target <target> --release
        ```
        The resulting application binary is located in
        ```
        target/<target>/release/helium_gateway
        ```

    2. Build an application package using one of the target system profile names
        ```shell
        cargo make --profile <platform> pkg
        ```
        The resulting `ipk` or `deb` will be located in
         ```
         target/ipk/helium-gateway-<version>-<platform>.<ipk or deb>
         ```


