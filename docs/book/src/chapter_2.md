# Async firmware structure

Peripherals should be initialized in the main function, all embassy tasks should be defined in separate files with their corresponding structs and impls.

```rust
src
├── main.rs
├── task_uart.rs
├── task_wifi.rs
└── task_net.rs
(...)
```

Each task is defined in its own file, allowing for modularity and separation of concerns. The main function initializes the necessary peripherals and spawns the tasks using the Embassy executor.

Most HALs provide isolated peripheral-oriented examples but no overarching (embassy-based) **project structure that discourages all-in-main.rs dog breakfast anti-pattern**: excuse my wording here but I hope the message gets through ;)

Following this structure might seem trivial, but here is where most of the architecture and design decisions start to come into play, such as:

- How to handle shared state between tasks such as critical boot and runtime configuration?
- Are tasks supposed to be cancelable? Restartable? Resumable?
- How to handle errors and recover from failures? Which crate provides the smallest RAM/Flash footprint for error handling?

Furthermore, if we are to [decouple IO from compute][sans_io_premise] for easy testing [and also timing][abstracting_time_sansio], which are the [main tenets of a SansIO approach][sans_io_ssh_stamp], we need to consider how [tasks and their state interact with a FSM][fsm_std_tests].


[sans_io_ssh_stamp]: https://github.com/brainstorm/ssh-stamp/issues/25
[abstracting_time_sansio]: https://www.firezone.dev/blog/sans-io#abstracting-time
[sans_io_premise]: https://www.firezone.dev/blog/sans-io#the-premise-of-sans-io
[fsm_std_tests]: https://github.com/brainstorm/ssh_fsm_tests