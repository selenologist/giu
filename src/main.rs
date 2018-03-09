#![feature(box_syntax)]

extern crate notify;
extern crate serde;
extern crate rmp_serialize;
extern crate rmp_serde;
extern crate rmpv;
extern crate rmp_rpc;
//extern crate websocket as ws;
extern crate hyper;
extern crate tokio_core;
extern crate tokio;
extern crate tokio_io;
extern crate futures;
extern crate subprocess;
extern crate regex;

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate lazy_static;

macro_rules! debug {
    ($fmt:expr)              => { eprintln!(concat!("[{:^15}] ", $fmt), ::std::thread::current().name().unwrap_or("unknown")) };
    ($fmt:expr, $($arg:tt)*) => { eprintln!(concat!("[{:^15}] ", $fmt), ::std::thread::current().name().unwrap_or("unknown"), $($arg)*) }
}

mod rebuilder;
mod graph;
//mod websocket;
mod http;
mod noderpc;
mod file;

fn main(){
    let (rebuilder, invalidation_rx) = rebuilder::launch_thread();
    let http      = http::launch_thread(invalidation_rx);
    debug!("threads launched, waiting for join");
    http.join().unwrap();
    rebuilder.join().unwrap();
}
