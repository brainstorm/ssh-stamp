```mermaid
gantt
    title SSH Stamp (a.k.a esp-ssh-rs) development plan under NLNet grant
    dateFormat YYYY-MM-DD
    excludes weekends
    tickInterval 1day
    weekday monday
    todayMarker off
    axisFormat %e
    section Prototype
        UART <-> SSH working    :      active, uart_ssh, 2025-04-01, 24h
    section Provisioning
        Provisioning            :      prov, after uart_ssh, 16h
        OTA updates             :      ota, after prov, 12h
    section Docs
        usage docs              :      usage_docs, after ota, 4h
        dev docs                :      dev_docs, after ota, 2h
    section Robustness
        #forbid(unsafe)         :      no_unsafe, after ota, 12h
        UART perf               :      uart_intr, after uart_ssh, 12h
        sans-io refactor        :      sans_io, after uart_intr, 16h
    section Multi-target
        Espressif chips         :      all_espressif, after no_unsafe, 12h
        Other chip1             :      chip1, after all_espressif, 20h
        Other chip2             :      chip2, after chip1, 18h
    section Testing
        CI/CD                   :      ci, after chip2, 16h
        hardware in test loop   :      HIL, after ci, 21h
        Users test              :      user_tests, after dev_docs, 9h
    section Security
        Self audit              :      self_sec_audit, after all_espressif, 10h
        NLNet security audit?   :      nlnet_sec_audit, after all_espressif, 45h
```

```verbatim
Total hours: 200h
Hourly rate: 40 eur/h
```

<!--
Original email snippet about cost estimates (as sent to NLNet on 2025-03-03)

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

TOTAL: 5000€ (wrong estimate!?)
-->