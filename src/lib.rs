#![feature(slice_patterns)]
#![feature(plugin)]
#![plugin(clippy)]

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;

extern crate rand;
extern crate quickcheck;
extern crate rustc_serialize;
extern crate bincode;
extern crate termion;
extern crate protobuf;
extern crate rsdb;

mod mindmap;
mod meta;
mod logging;
mod plot;
mod task;
mod pb;

pub use mindmap::{serialize_screen, deserialize_screen, Screen, init_screen_log};
