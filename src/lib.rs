#![no_std]
#![no_main]
// #![forbid(unsafe_code)]
#[deny(clippy::mem_forget)] // avoids any UB, forces use of Drop impl instead
pub mod config;
pub mod errors;
pub mod espressif;
pub mod keys;
pub mod serial;
pub mod serve;
pub mod settings;
pub mod storage;
