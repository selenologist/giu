#![feature(box_syntax)]
#![feature(conservative_impl_trait)]
#![feature(try_trait)]
#![allow(dead_code)]
extern crate notify;
extern crate serde;
extern crate serde_json;
//extern crate rmp_serde;
//extern crate rmpv;
extern crate ws;
extern crate hyper;
extern crate tokio_core;
extern crate tokio;
extern crate tokio_io;
extern crate futures;
extern crate subprocess;
extern crate regex;
extern crate time;

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate log;
extern crate env_logger;

macro_rules! profile {
    ($name:expr, $bl:block) => ({
        use log::Level::Trace;
        if log_enabled!(Trace){
            let name   = $name;
            let start  = time::PreciseTime::now();
            let result = $bl;
            trace!("{} took {:?}", name, start.to(time::PreciseTime::now()));
            result
        }
        else{
            $bl
        }
    })
}

mod rebuilder;
mod graph;
mod websocket;
mod http;
mod file;
mod filecache;
mod filethread;
mod reloader;

use rebuilder::InvalidationReceiverChain;

fn main(){
    // configure logger    
    env_logger::Builder::from_default_env()
        .format(|buf, record| {
            use std::io::Write;
            use log::Level;
            use env_logger::Color;
            use std::cmp::min;

            let write_level = {
                // pulled from env_logger 0.5.5 default logger
                let level = record.level();
                let mut level_style = buf.style();

                match level {
                    Level::Trace => level_style.set_color(Color::White),
                    Level::Debug => level_style.set_color(Color::Blue),
                    Level::Info => level_style.set_color(Color::Green),
                    Level::Warn => level_style.set_color(Color::Yellow),
                    Level::Error => level_style.set_color(Color::Red).set_bold(true),
                };

                write!(buf, "{:>5}", level_style.value(level))
            };
            
            let write_thread = {
                let mut style = buf.style();
                style.set_color(Color::Magenta);
                write!(buf,
                       "[{:>12}!",
                       style.value(std::thread::current().name().unwrap_or("unknown")))
            };

            // if the path root is the main package's name, strip it
            let module_path = record.module_path().unwrap_or("unknown");
            let write_path = {
                static MAIN_NAME: &'static str = env!("CARGO_PKG_NAME");
                let min_len = min(module_path.len(), MAIN_NAME.len());
                let (style, path) =
                    if module_path[0..min_len] == MAIN_NAME[..]{
                        let mut style = buf.style();
                        style.set_bold(true);
                        let path = if module_path.len() >= min_len+2{
                                &module_path[min_len+2..] // strip :: if present
                            }
                            else{
                                "main"
                            };
                        (style, path)
                    }
                    else{
                        (buf.style(), &module_path[..])
                    };
                write!(buf, "{:<12}] ", style.value(path))
            };

            let write_args =
                writeln!(buf, "{}", record.args());

            write_level.and(write_thread).and(write_path).and(write_args)
        })
        .init();

    // start threads
    let (rebuilder, invalidation_rx) = rebuilder::launch_thread();
    let (invalidation_chain, invalidation_rx) =
        InvalidationReceiverChain::with_daisy(invalidation_rx);
    let cache     = filecache::FileCache::new(4, invalidation_chain);
    let http      = http::launch_thread(cache);
    let websocket = websocket::launch_thread();    
    let reloader  = reloader::launch_thread(invalidation_rx.into());
    debug!("Threads launched, waiting for join");
    reloader.join().unwrap();
    websocket.join().unwrap();
    http.join().unwrap();
    rebuilder.join().unwrap();
}
