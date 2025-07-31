# Async firmware structure

Peripherals should be initialized in the main function [and shared between tasks][embassy_shared_peripherals_pattern], all embassy tasks should be defined in separate files with their corresponding structs and impls.

```rust
% tree
.
├── lib.rs
├── main.rs
└── tasks
    ├── mod.rs
    ├── net.rs
    ├── uart.rs
    └── wifi.rs
(...)
```

Each task is defined in its own file, allowing for modularity and separation of concerns. Ideally, initialization of a peripheral and its long-running form should be separate, i.e:

```rust
(...)
    spawner.spawn(tasks::uart::init(peripherals)).unwrap();
    spawner.spawn(tasks::uart::run()).unwrap();

    spawner.spawn(tasks::wifi::init(peripherals)).unwrap();
    spawner.spawn(tasks::wifi::run()).unwrap();
(...)
```

The main function initializes the necessary peripherals and spawns the tasks using the Embassy executor. But in the snippet above some problems start to arise: 

1. Should we pass around `esp_hal::peripherals::Peripherals`?
1. Or perhaps just initialise the concrete peripherals we need such as `peripheral.UART1` being then passed to the UART task only?
1. How do we "pack" groups of required peripherals and pass them to the task? Espressif wireless typically requires `peripherals.{WIFI && TIMG0 && RNG && SYSTIMER}`
1. How do we "unpack" (split AND share) `peripheral.RNG` if we later want to have a task that deals with cryptography (which will require random number generation via the RNG peripheral)?
1. Can we safely free up unused heap/stack space from, `tasks::wifi::init()` after it has done its job? As opposed to `tasks::wifi::run()`, it's a one-off task... perhaps it should be a free-standing function instead?

Most HALs provide isolated peripheral-oriented examples but no overarching (embassy-based) **project structure that discourages all-in-main.rs anti-pattern**.

Following this structure might seem trivial, but here is where most of the architecture and design decisions start to come into play, such as:

- How to handle shared state between tasks such as critical boot and runtime configuration?
- Are tasks supposed to be cancelable? Restartable? Resumable?
- How to handle errors and recover from failures? Which crate provides the smallest RAM/Flash footprint for error handling?

Furthermore, if we are to [decouple IO from compute][sans_io_premise] for easy testing [and also timing][abstracting_time_sansio], which are the [main tenets of a SansIO approach][sans_io_ssh_stamp], we need to consider how [tasks and their state interact with a FSM][fsm_std_tests].


[sans_io_ssh_stamp]: https://github.com/brainstorm/ssh-stamp/issues/25
[abstracting_time_sansio]: https://www.firezone.dev/blog/sans-io#abstracting-time
[sans_io_premise]: https://www.firezone.dev/blog/sans-io#the-premise-of-sans-io
[fsm_std_tests]: https://github.com/brainstorm/ssh_fsm_tests
[embassy_shared_peripherals_pattern]: https://embassy.dev/book/#_sharing_peripherals_between_tasks
