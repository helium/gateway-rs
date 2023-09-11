# Helium Gateway

[![ci](https://github.com/helium/gateway-rs/workflows/ci/badge.svg)](https://github.com/helium/gateway-rs/actions)

The Helium Gateway application is a service designed to run on Linux-based
LoRaWAN gateways.

It's intended to run alongside a typical LoRa packet forwarder and to connect
via Semtech's Gateway Messaging Protocol (GWMP, using JSON v1 or v2).

In turn, the Helium Gateway application does two things:

- Periodically sends and witnesses Proof of Coverage beacons, which are reported
  to the Proof of Coverage ingest oracles
- connects and routes packets to the Helium Packet Router

```
                                                                 +-----------+
+-----------+                       +------------+               |  Ingest   |
|           |                       |            |<--- gRPC ---->|  Service  |
|  packet   |<--- Semtech GWMP ---->|   Helium   |               +-----------+
| forwarder |       over UDP        |   Gateway  |               +-----------+
|           |                       |            |<--- gRPC ---->|  Helium   |
+-----------+                       +------------+               |  Router   |
                                                                 +-----------+
```

**NOTE**: A DIY Helium Gateway based hotspot is eligible for data rewards **only**.
Proof of coverage rewards are only possible for [approved maker hotspots](https://github.com/dewi-alliance/hotspot-manufacturers).

### Releases

The project builds binary compressed tar
[release](https://github.com/helium/gateway-rs/releases) files which are named
after the crypto module used and the CPU architecture they were built for. For example, `helium-gateway-1.0.0-aarch64-unknown-linux-gnu.tar.gz` contains the
`helium_gateway` executable with ecc608 support and its `setttings.toml`
configuration file.

For versions using the ecc608, the crypto module name is not included in the
file name. For TPM variants it is included, for example,
`helium-gateway-1.0.0-x86_64-tpm-debian-gnu.tar.gz`

Releases are tagged using [semantic versioning](https://semver.org) with a
`major.minor.patch` form. An alpha/beta release tag may also be issued for early
feature/bug development and testing. Makers are _not_ required to pick up
alpha/beta releases.

## Linux Dependencies

This application requires a Linux-based environment for the following reasons:

- `tokio`: the `gateway-rs` application, written in Rust, depends on
  [Tokio](https://docs.rs/tokio) for its runtime. Tokio binds to Linux
  interfaces such as `epoll` and `kqeueue`. It is technically possible to port
  Tokio to another OS or RTOS (this has been done for Windows), but it would be
  no simple undertaking.

## Installing

If your [supported LoRa gateway](#supported-platforms) did not come with
helium-gateway pre-installed, manual installation requires you to:

1. Configure the packet forwarder on the gateway to forward to the
   helium-gateway application. This varies per gateway but the goal is to set
   the packet forwarder to forward to the (default) configured helium-gateway on
   `127.0.0.1` at udp port `1680`
2. Set up ssh acccess to the gateway. Depending on the gateway that may require
   going through a web interface, while others already have ssh configured.
3. `scp` a downloaded and uncompressed release package for the supported
   platform to the gateway. e.g.
   ```shell
   scp helium_gateway settings.toml <gateway>:/tmp/
   ```
4. `ssh` into the device and copy the application and configuaration into a
   suitable location using a command like:

   ```shell
   mkdir /etc/helium_gateway
   mv /tmp/settings.toml /etc/helium_gateway/
   mv /tmp/helium_gateway /usr/bin/
   ```

5. Configure the logging method to use by updating the `settings.toml` file's
   `[log]` section with the logging method to use based on your system.
   Supported values are `stdio` or `syslog`. Note you may need to configure
   the `syslog` service on your device to accept the logs.

6. Configure the region if required. The default region of the gateway is set to
   `UNKNOWN`, and fetched based on the asserted location of the gateway. Setting
   the region to a known region or caching the last fetched region and using
   the `GW_REGION` environment variable on startup will allow the gateway to use
   the correct region for uplinks immediately, while the region parameters are
   retrieved.

   The supported region values are listed in the [region protobuf definition](https://github.com/helium/proto/blob/master/src/region.proto).

   **NOTE**: Due to TX power regulations, the gateway location needs to be
   asserted on the blockchain to be able to send downlinks.

7. Start the service by either starting it manually or hooking it into the
   `init.d`, `systemd`, or equivalent system services for your platform. Consult
   your platform/linux on how best to do this.

   The startup command for the application is as follows. Note you will need to
   adjust the path to `helium_gateway` or the path to the settings file to use
   for the `-c` option.

   ```shell
   /usr/bin/helium_gateway -c /etc/helium_gateway/settings.toml server
   ```

If this command succeeds the logs on the gateway will show the service starting
and the local packet forwarder client connecting to the gateway service.

## Supported Targets

The following targets are being built. Adding a new target involves creating a
pull request against this repository with the right cpu target tuple.

- Review [the open issues](https://github.com/helium/gateway-rs/issues) to see
  if it's already in progress. If not, file an issue. Note that new targets are
  developed and supported by Helium makers and Community members.
- Join the `#gateway-development` channel on [Helium
  Discord](https://discord.gg/helium) and work the the community to add target
  support.

Note that platforms will be tested much faster if you join the development process!

| Target                         | Products                                                                                                                    |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| mipsel-unknown-linux-musl      | :white_check_mark: CalDigit Light Hotspot                                                                                   |
|                                | :white_check_mark: ClodPi Light Hotspot [ClodPi]                                                                            |
|                                | :white_check_mark: [RAK833] EVB Kit                                                                                         |
|                                | :white_check_mark: [RAK7258] (WisGate Edge Lite)                                                                            |
|                                | :grey_question: [RAK7249] (WisGate Edge Max)                                                                                |
|                                | :white_check_mark: [RAK7240] (WisGate Edge Prime)                                                                           |
|                                | :white_check_mark: Smart Harvest Instruments Light Gateway                                                                  |
| mips-unknown-linux-musl        | :white_check_mark: Dragino [LPS8]                                                                                           |
|                                | :grey_question: Dragino [DLOS8]                                                                                             |
| aarch64-unknown-linux-gnu      | :white_check_mark: Cotx gateways [cotx]                                                                                     |
|                                | :white_check_mark: Raspberry Pi 3 or 4 running Raspian / Raspberry Pi OS 64 bit or another 64 bit Debian-based Linux distro |
| arm-unknown-linux-gnueabihf    | :white_check_mark: Raspberry Pi 0 or 1 running Raspian / Raspberry Pi OS or another Debian-based Linux distro               |
| armv5te-unknown-linux-musleabi | :white_check_mark: CloudGate                                                                                                |
|                                | :white_check_mark: Multitech Conduit [MTCDT] (mLinux)                                                                       |
| armv7-unknown-linux-musleabihf | :white_check_mark: Kerlink [Wirnet iFemtoCell Evolution]                                                                    |
| armv7-unknown-linux-gnueabihf  | :white_check_mark: ResIOT X gateways [resiot]                                                                               |
|                                | :white_check_mark: Raspberry Pi 2, 3, or 4 running Raspian / Raspberry Pi OS or another Debian-based Linux distro           |
|                                | :white_check_mark: [Kona Micro] IoT Gateway                                                                                 |
|                                | :white_check_mark: [Kona Enterprise] IoT Gateway                                                                            |
|                                | :white_check_mark: [RisingHF RHF2S027] Light Hotspot                                                                        |
| x86_64-unknown-linux-gnu       | :white_check_mark: Debian x86_64 (ecc608)                                                                                   |
|                                | :white_check_mark: LongAP                                                                                                   |
| x86_64-tpm-debian-gnu          | :white_check_mark: Debian x86_64 (tpm)                                                                                      |
|                                | :white_check_mark: FreedomFi gateway                                                                                        |

[rak833]: https://github.com/RAKWireless/RAK2247-RAK833-LoRaGateway-OpenWRT-MT7628
[rak7258]: https://store.rakwireless.com/products/rak7258-micro-gateway
[rak7249]: https://store.rakwireless.com/products/rak7249-diy-outdoor-gateway
[rak7240]: https://store.rakwireless.com/products/rak7240-outdoor-lpwan-gateway?variant=36068284465310
[wirnet ifemtocell evolution]: https://www.kerlink.com/product/wirnet-ifemtocell-evolution/
[lps8]: https://www.dragino.com/products/lora-lorawan-gateway/item/148-lps8.html
[dlos8]: https://www.dragino.com/products/lora-lorawan-gateway/item/160-dlos8.html
[mtcdt]: https://www.multitech.com/brands/multiconnect-conduit
[resiot]: https://www.resiot.io/en/resiot-gateways/
[cotx]: https://www.cotxnetworks.com/product/service_one
[kona micro]: https://www.tektelic.com/catalog/kona-micro-lorawan-gateway
[kona enterprise]: https://www.tektelic.com/catalog/kona-enterprise-lorawan-gateway
[risinghf rhf2s027]: https://www.risinghf.com/product/detail/27
[clodpi]: https://clodpi.io
[cloudgate]: https://www.option.com/

## Building

Use one of the existing
[releases](https://github.com/helium/gateway-rs/releases) if you can, or build
your own by hand using the instructions below.

If you want to support a new target, please consider submitting a PR to get the
target as part of the supported matrix of gateway platforms!

1. Install `rust`
   ```shell
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
2. Install cargo `cross`. The `cross` command allows for cross-compiling to hardware targets using a docker image.
   ```shell
   cargo install cross
   ```
3. Build the application binary using the target triplet/profile from the
   supported targets. Note that debug builds may be to large to run on some
   targets.

   ```shell
   cross build --profile <target> --release build
   ```

   The resulting application binary is located in

   ```
   target/<target>/release/helium_gateway
   ```

   **NOTE** The target triplet and profile may not be the same. For example, the
   ` x86_64-tpm-debian-gnu` profile uses the `x86_64-unknown-linux-gnu` target

## Additional usage info

The Helium Gateway application can be configured to suit your hardware/software
environment in a variety of ways - either from the command line or customizations to the `settings.toml` file or with environment variables. The
following sections describe this functionality in more detail as well as more
general information on how to use the application.

### Settings file

The Helium Gateway application is configured using a TOML settings file. The
released settings file can be found in the
[settings.toml](https://github.com/helium/gateway-rs/blob/main/config/settings.toml)
file in this repo. Edit this file with specifics for your target platform and
store it either at the default expected
location`/etc/helium_gateway/settings.toml`or at a custom location of your
choosing. If you store the file in a non-default location you will need to pass
the`-c`flag to the`helium_gateway` application as shown below in the [general
usage section](#general-usage-info).

### Using the ECC crypto chip

If your gateway is enabled with an ECC608 crypto chip which is set up correctly,
you can configure helium_gateway to use the crypto chip for secure key storage
and crypto operations.

To use in your `settings.toml` override the `keypair` setting to reflect the use
of the ECC and specify the bus address and slot to use. For example:

```
keypair = "ecc://i2c-1:96?slot=0&network=mainnet"
```

will have helium_gateway use the ECC at the `/dev/i2c-1` device driver location,
use bus address `96` (which is hex `0x60`), and slot `0` for its crypto
operations. While marking the resulting key as a mainnet key. Bus address, slot
and network are all optional parameters and default to the above values (only
device driver location is required such as  `ecc://i2c-1`).

Note that the file-based keypair will no longer be used once the ECC is
configured for use.

See the [gateway-mfr-rs repo](https://github.com/helium/gateway-mfr-rs) for
instructions on configuring, locking, and testing an ECC chip.

It is expected that most gateways will use the same key slot for the onboarding key and the keypair, however, this key is also configurable in the same way as the keypair:

```
onboarding = "ecc://i2c-1:96?slot=0"
```

The original helium miners use an onboarding key on slot 15:

```
onboarding = "ecc://i2c-1:96?slot=15"
```

### Envrionment variables

Instead of editing parameters in the
[settings.toml](https://github.com/helium/gateway-rs/blob/main/config/settings.toml)
file as described above, you can also use environment variables. The environment
variable name will be the same name as the entries in the settings file in
uppercase and prefixed with "GW\_". For example, following on from the above
example where we change the region using `region = "EU868"` in the settings
file, setting an environment variable of `GW_REGION="EU868"` will override the
region setting. If the settings are in one of the lower sections such as the
`[log]` section then you need to also include that in the environment variable
name such as `GW_LOG_LEVEL`.

The settings are loaded first from the `settings.toml` file, and then from
environment variables and any duplicates are overridden in the order. Therefore,
please note that if you have a setting in both locations, the environment
variable will override the settings in the other two locations.

### General usage info

Using the Helium Gateway application is pretty simple, and the vast majority of
the information you will need to use it can be gleaned by using the `--help`
flag which provides the following output:

```
Helium Light Gateway

USAGE:
    helium_gateway [FLAGS] [OPTIONS] <SUBCOMMAND>

FLAGS:
        --daemon     Daemonize the application
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -c <config>        Configuration file to use [default: /etc/helium_gateway/settings.toml]

SUBCOMMANDS:
    add       Construct an add gateway transaction for this gateway
    help      Prints this message or the help of the given subcommand(s)
    info      Info command. Retrieve all or a subset of information from the running service
    key       Commands on gateway keys
    server    Run the gateway service
```

As you can see, apart from the `help` command, there are four core subcommands
that you can pass: `add`, `key`, `server`. The descriptions of what these
subcommands do is shown in brief in the above help output, and are explained in
more detail in the sections below.

The only option available is the `config` option using the `-c` flag. This tells
the application where your configuration file is located and can be used as
follows whilst passing any of the other commands such as `server` or `add`
(default is `/etc/helium_gateway/settings.toml`):

```
./helium_gateway -c /location/of/config/file server
```

Lastly you can check the version, read the help information, or daemonize the
application using the `--version`, `--help` and `--daemon` flags respectively.

### Add gateway subcommand

As shown in the help output below, this subcommand is used to construct an add
gateway transaction which can subsequently be used with the Helium Wallet
application to onboard the gateway to the blockchain. More information on this
process can be found [on the docs article for Data Only
Hotspots](https://docs.helium.com/mine-hnt/data-only-hotspots/#add-hotspot).

```
Construct an add gateway transaction for this gateway

USAGE:
helium_gateway add [OPTIONS] --owner <owner> --payer <payer>

FLAGS:
-h, --help Prints help information
-V, --version Prints version information

OPTIONS:
--mode <mode> The staking mode for adding the light gateway [default: dataonly]
--owner <owner> The target owner account of this gateway
--payer <payer> The account that will pay account for this addition
```

So for example, to construct a data-only add gateway transaction you would enter
the following command at the terminal:

```
./helium_gateway add --owner WALLET_ADDRESS --payer WALLET_ADDRESS
```

You need to substitute WALLET_ADDRESS for the wallet address you will use for
the owner of the hotspot and the payer of the transaction fees
respectively...but please note that the `--payer` address must be the same as
the one you will use to submit the transaction to the blockchain, or the
transaction will fail.

The output of this command is a JSON object which looks like the following:

```json
{
  "address": "11TL62V8NYvSTXmV5CZCjaucskvNR1Fdar1Pg4Hzmzk5tk2JBac",
  "fee": 65000,
  "mode": "dataonly",
  "owner": "14GWyFj9FjLHzoN3aX7Tq7PL6fEg4dfWPY8CrK8b9S5ZrcKDz6S",
  "payer": "14GWyFj9FjLHzoN3aX7Tq7PL6fEg4dfWPY8CrK8b9S5ZrcKDz6S",
  "staking fee": 1000000,
  "txn": "CrkBCiEBrlImpYLbJ0z0hw5b4g9isRyPrgbXs9X+RrJ4pJJc9MkSIQA7yIy7F+9oPYCTmDz+v782GMJ4AC+jM+VfjvUgAHflWSJGMEQCIGfugfLkXv23vJcfwPYjLlMyzYhKp+Rg8B2YKwnsDHaUAiASkdxUO4fdS33D7vyid8Tulizo9SLEL1lduyvda9YVRCohAa5SJqWC2ydM9IcOW+IPYrEcj64G17PV/kayeKSSXPTJOMCEPUDo+wM="
}
```

You can also pass a `--mode` flag followed by the hotspot type (`dataonly |
light | full`) as shown below:

```
./helium_gateway add --owner WALLET_ADDRESS --payer WALLET_ADDRESS --mode light
```

The output of this command will be mostly the same as if you used the default
`dataonly` however you will see that the mode has changed to `"mode": "light",`
and the staking fee amount has changed to `"staking fee": 4000000`.

The ` txn` field from the JSON object needs to be used as the input to the wallet
command `helium-wallet hotspot add` when you subsequently want to add it to the
blockchain. For example, using the above JSON object as an example, you would
use the following command:

```
helium-wallet hotspots add CrkBCiEBrlImpYLbJ0z0hw5b4g9isRyPrgbXs9X+RrJ4pJJc9MkSIQA7yIy7F+9oPYCTmDz+v782GMJ4AC+jM+VfjvUgAHflWSJGMEQCIGfugfLkXv23vJcfwPYjLlMyzYhKp+Rg8B2YKwnsDHaUAiASkdxUO4fdS33D7vyid8Tulizo9SLEL1lduyvda9YVRCohAa5SJqWC2ydM9IcOW+IPYrEcj64G17PV/kayeKSSXPTJOMCEPUDo+wM=
```

### Gateway keys subcommand

This subcommand can be used to get the address and animal name of the gateway
from the keys file as shown in the help output below. Note that the
helium_gateway server has to be running for this command to work.

```
Commands on gateway keys

USAGE:
    helium_gateway key <SUBCOMMAND>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    help    Prints this message or the help of the given subcommand(s)
    info    Commands on gateway keys
```

Using this is as simple as passing the following command in a terminal from
wherever you installed the `helium_gateway` application:

```
./helium_gateway key info
```

The output of this is a JSON object containing the address and animal name of
the hotspot as shown below:

```json
{
  "address": "11TL62V8NYvSTXmV5CZCjaucskvNR1Fdar1Pg4Hzmzk5tk2JBac",
  "name": "wide-neon-kestrel"
}
```

### Gateway server

The gateway server subcommand is used to start the gateway service on your
device.

```
Run the gateway service

USAGE:
    helium_gateway server

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information
```

Running it is as simple as:

```
./helium_gateway server
```

However as discussed above you can also pass the `-c` option to tell the service that you are using a different location for your config files:

```
./helium_gateway -c /location/of/config/file server
```

## More docs and info

There is a wealth of further information on maker hotspot software on the [Helium Docs site](https://docs.helium.com/solana/migration/maker-hotspot-software/) including information about the [gRPC API](https://github.com/helium/gateway-rs/tree/main/src/api) that allows you to interact with the gateway via the maker app and other services over gRPC rather than via the command line options described above.
