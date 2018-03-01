extern crate notify;
extern crate serde;
extern crate rmp_serialize;
extern crate rmp_serde;
extern crate rmpv;
#[macro_use]
extern crate serde_derive;
extern crate rmp_rpc;
extern crate ws;
extern crate hyper;
extern crate hyper_staticfile;
extern crate tokio_core;
extern crate futures;
extern crate subprocess;

mod rebuilder;
mod graph;
mod websocket;
mod http;
mod noderpc;

fn main(){
    let rebuilder = rebuilder::launch_thread();
    let websocket = websocket::launch_thread();
    let http      = http::launch_thread();
    rebuilder.join().unwrap();
    websocket.join().unwrap();
    http.join().unwrap();
}
