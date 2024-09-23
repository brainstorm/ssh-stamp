# Motivation

This project aims to have a secure SSH-to-UART bridge for embedded microcontrollers.

The expected outcomes are:

1. To have a WiFi AP/STA device that a user can SSH into and securely manage any other device with an UART.
1. The device should be relatively effortless to deploy and provision with the required secret key material.
1. Written in embedded Rust (ideally `no_std` and `no alloc` to reduce memory fragmentation and allow long runtimes without memory issues).
1. To initially target Espressif's ESP32 microcontrollers, but extend it to other devices such as Sipeed Maix M0sense, BouffaloLab's BL6xx, Realtek Ameba IOT SoC series, etc... 
1. Eventually generalise towards a reusable and multi-target embedded-ssh-rs (generic) Rust project.

# Similar past projects

I worked with [Crypt4GH-rust][crypt4gh-rust], a Rust implementation of an encryption scheme ([Crypt4GH][crypt4gh]) involving `X25519_chacha20_ietf_poly1305` encryption/decryption to assist large human genomics datasets (DNA sequencing data encryption).

My contributions were aimed at refactor out the bindings with a deprecated libsodium bindings C library towards RustCrypto (pure Rust cryptographic primitives) and also rewrite its public API into a more idiomatic and user friendly version.

I've also been featured in the Rust Embedded showcase via the [bbtrackball-rs project (a.k.a "Rusty Mouse)][rust-embedded-showcase].

# Requested support

I would like to request 8000eur to attain the first 3 milestones for this project. The last two milestones should be evaluated carefully in future grant applications due to its significantly increased workload, scope and ambitions (target architecture might not have mature enoght Rust and/or cryptographic support infrastructure).

The costs are mainly firmware writing labour and very specialised consultant contracts with at least one more professional embedded firmware developer for ~10 months at a competitive hourly rate. Hardware and test equipment costs are already covered. Also, local meetings, presentations and related travel is also well established and does not incur a significant extra expense, so we can absorb that cost.

# Comparison with the state of the art

> Compare your own project with existing or historical efforts.

1. NanoKVM is a recent Keyboard-Video-Monitor system with much higher complexity and [attack surface][nanokvm-security] than this project. Also [closed source at the time of writing this](https://github.com/sipeed/NanoKVM/issues/1#issuecomment-2246900903).

1. Espressif's [esp-hosted][esp-hosted] provides a similar solution written in C that does not have the SSH component described here. A open WiFi network or a locally shared wireless LAN would be open to attack by "passers-by" (that have the correct wifi Pre-Shared Key, for WPA).

1. [esp-link][esp-link], a WiFi-UART serial bridge from JEELABS, suffers from the same aforementioned security limitation, also it's not a memory safe firmware.

# Technical challenges

> What are significant technical challenges you expect to solve during the project, if any?)

Rust embedded and especially [esp-rs/esp-hal][esp-hal], which is what this project relies upon to begin with, are in a relatively young state of maturity.

# Ecosystem

> Describe the ecosystem of the project, and how you will engage with relevant actors and promote the outcomes?

There's a vibrant community around Rust embedded community and also vendors such as Espressif have embraced Rust as well. Other open source communities such as OpenWrt can be very positively benefited from the "WiFi-SSH-UART co-processor" outlined here. Among other devices, routers such as the BananaPi BPI-R3 can be an excellent showcase for this project.

[crypt4gh]: https://samtools.github.io/hts-specs/crypt4gh.pdf
[crypt4gh-rust]: https://github.com/EGA-archive/crypt4gh-rust/
[nanokvm]: https://github.com/sipeed/NanoKVM/issues/1#issuecomment-2246900903
[nanokvm-security]: https://lichtlos.weblog.lol/2024/08/how-to-reverse-the-sipeed-nanokvm-firmware
[esp-hosted]: https://github.com/espressif/esp-hosted
[esp-link]: https://github.com/jeelabs/esp-link
[esp-hal]: https://github.com/esp-rs/esp-hal
[rust-embedded-showcase]: https://showcase.rust-embedded.org/
