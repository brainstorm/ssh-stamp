#![no_std]
#![no_main]
//#![forbid(unsafe_code)]
#![feature(type_alias_impl_trait)]

pub mod settings;
pub mod io;
pub mod serve;
pub mod esp_rng;
pub mod esp_net;
pub mod keys;