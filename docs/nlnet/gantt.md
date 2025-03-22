<!--
 1. To have a WiFi AP/STA device that a user can SSH into and securely manage any other device with an UART.
    1.1 Prototype cost me (out of pocket) around 500€, needs more refinement, so probably should cost no more than 900€ at this point.

2. The device should be relatively effortless to deploy and provision with the required secret key material.
    2.1 Challenging as there are as many ways to onboard devices as opinions about it. But applying simplicity and involving third parties in testing, I'd budget this at an additional 800€.

3. Written in embedded Rust (ideally no_std and no alloc to reduce memory fragmentation and allow long runtimes without memory issues).
    3.1 Many of the unsafe issues have been circumvented, but way more work is needed to make this robust.
    3.2 Espressif UART-DMA serial driver vs Interrupt driver: Implementing the most suitable solution that does not overrun or glitch the UART (has happened), ~700€
    3.3 Porting to as many Espressif targets as possible, taking care of memory requirements and setting up HIL (Hardware In the Loop) testing jigs: ~800€
    3.4 [Sans-IO refactor][sans-io]: The current prototype needs a cleaner decoupling of finite state machines and IO, but a careful focus on performance, ~1500€
    3.5 Run SSH audit with specialised tools such as SSHambles by HDmoore et al: 300€

TOTAL: 5000€
-->

```mermaid
gantt
    title SSH Stamp development plan under NLNet grant
    dateFormat YYYY-MM-DD
    excludes weekends
    tickInterval 1month
    weekday monday
    todayMarker off
    axisFormat %Y-%m-%d
    section Prototype
        UART <-> SSH working    :      active, uart_ssh, 2025-04-01, 60d
    section Provisioning
        Provisioning            :      prov, after uart_ssh, 20d
        OTA updates             :      ota, after prov, 30d
    section Docs
        usage docs              :      usage_docs, after ota, 20d
        dev docs                :      dev_docs, after ota, 15d
    section Robustness
        #forbid(unsafe)         :      no_unsafe, after ota, 20d
        UART interrupts         :      uart_intr, after uart_ssh, 30d
    section Multi-target
        Other espressif targets :      all_espressif, after no_unsafe, 30d
        Other chip1             :      chip1, after all_espressif, 30d
        Ohter chip2             :      chip2, after chip1, 30d
    section Testing
        ci                      :      ci, after chip2, 7d
        hardware in the loop    :      HIL, after ci, 25d
        user_testing            :      user_testing, after dev_docs, 30d
    section Security
        Self security audit     :      self_sec_audit, after all_espressif, 25d
        NLNet security audit    :      nlnet_sec_audit, after all_espressif, 60d
```

Total hours: 200h

Hourly rate: 40 eur/h