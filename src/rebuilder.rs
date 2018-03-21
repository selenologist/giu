use notify::{DebouncedEvent, Watcher, RecursiveMode, watcher};
use subprocess::{Exec, ExitStatus};

use bus::{BusReader, Bus};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel};
use std::time::Duration;
use std::io;
use std::fs;
use std::path::{Path};
use std::thread;
use std::thread::{JoinHandle};

pub type InvalidatedPath = String;
pub type InvalidatedReceiver = BusReader<InvalidatedPath>;

type InvalidatedSender = Arc<Mutex<Bus<InvalidatedPath>>>;

static INVALIDATION_BUS_SIZE: usize = 16; // bounded number of unreceived elements on file invalidation bus, increase if necessary but this should be plenty

// BusReader cannot be cloned so we have to keep hidden access to the bus itself
// to be able to make more BusReaders elsewhere in the program.
// This is so not ideal but it'll do
pub struct InvalidatedReceiverMaker{
    bus: InvalidatedSender
}

impl InvalidatedReceiverMaker{
    pub fn add_rx(&self) -> InvalidatedReceiver{ // will block on mutex lock
        self.bus.lock().unwrap().add_rx()
    }
}

fn process_coffee(path: &Path) -> io::Result<()>{
    info!("Compiling {}", path.to_str().unwrap());
    let exit_status = match
        Exec::cmd("coffee")
        .arg("-c")
        .arg(path)
        .join(){
            Ok(k) => k,
            Err(e) => return Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))
    };

    if exit_status != ExitStatus::Exited(0){
        error!("Error, returned {:?}", exit_status);
        Err(io::Error::new(io::ErrorKind::Other, format!("exited with status {:?}", exit_status)))
    }
    else{
        info!("{} processed.", path.to_str().unwrap());
        Ok(())
    }
}

fn check<P: AsRef<Path>>(path: P){
    match path.as_ref().extension(){
        Some(ext) if ext == "coffee" => {
            match process_coffee(path.as_ref()){
                Ok(..) => {},
                Err(e) => error!("Failed to process: {:?}", e)
            }
        },
        _ => {}
    }
}


fn recursive_find(path: &Path) -> io::Result<()>{
    trace!("Entering {}", path.to_str().unwrap());
    for p in fs::read_dir(path)?{
        let e = p.unwrap();
        info!("Found file {:?}", e.file_name());
        if e.file_type().unwrap().is_dir(){
            recursive_find(&e.path())?;
        }
        if e.file_type().unwrap().is_file(){
            let path = e.path();
            check(path)
        }
    }
    Ok(())
}

fn handle_event(event: DebouncedEvent, invalidation_tx: &InvalidatedSender){
    use self::DebouncedEvent::*;

    // make a closure and reborrow even in Rename to prevent lock being held
    // across expensive functions like process_coffee
    // should only fail if this thread holds the lock which should never happen
    let broadcast = |s| invalidation_tx.lock().unwrap().broadcast(s);
    match event{
        Create(p) |
        Write(p)  => {
            let s = String::from(p.to_str().unwrap());
            info!("File {} modified", s);
            check(&p);
            broadcast(s);
        },
        Rename(old, new) => {
            let s1 = String::from(old.to_str().unwrap());
            let s2 = String::from(new.to_str().unwrap());
            info!("File {} renamed to {}", s1, s2);
            broadcast(s1);
            check(&new);
            broadcast(s2);
        },
        Remove(p) => {
            let s = String::from(p.to_str().unwrap());
            info!("File {} removed", s);
            broadcast(s);
        }
        _ => ()
    }
}

pub fn launch_thread() -> (JoinHandle<()>, InvalidatedReceiverMaker){
    // must be Arc<Mutex<Bus>> so that InvalidatedReceiverMaker can add more receivers
    let invalidation_tx = Arc::new(Mutex::new(Bus::new(INVALIDATION_BUS_SIZE)));
    let invalidation_rx_maker = 
        InvalidatedReceiverMaker{ bus: invalidation_tx.clone() };
    
    let handle = thread::Builder::new()
        .name("rebuilder".into())
        .spawn(move ||{
        let watch_path = "client/";
        trace!("Finding and processing existing .coffee files");
        recursive_find(Path::new(watch_path)).unwrap();

        let (watcher_tx, watcher_rx) = channel();

        let mut watcher = watcher(watcher_tx, Duration::from_secs(1)).unwrap();

        watcher.watch(watch_path, RecursiveMode::Recursive).unwrap();

        loop{
            match watcher_rx.recv(){
                Ok(ev) => handle_event(ev, &invalidation_tx),
                Err(e) => error!("watch error: {:?}", e)
            }
        }
    }).unwrap();

    (handle, invalidation_rx_maker)
}
