use notify::{DebouncedEvent, Watcher, RecursiveMode, watcher};
use subprocess::{Exec, ExitStatus};

use std::sync::mpsc::{Sender, Receiver, channel};
use std::time::Duration;
use std::io;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::thread::{JoinHandle};

pub type InvalidatedPath = String;
pub type InvalidatedReceiver = Receiver<InvalidatedPath>;

fn process_coffee(path: &PathBuf) -> io::Result<()>{
    debug!("Compiling {}", path.to_str().unwrap());
    let exit_status = match
        Exec::cmd("coffee")
        .arg("-c")
        .arg("-M")
        .arg(path)
        .join(){
            Ok(k) => k,
            Err(e) => return Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))
    };

    if exit_status != ExitStatus::Exited(0){
        debug!("Error, returned {:?}", exit_status);
        Err(io::Error::new(io::ErrorKind::Other, format!("exited with status {:?}", exit_status)))
    }
    else{
        debug!("{} processed.", path.to_str().unwrap());
        Ok(())
    }
}

fn recursive_find(path: &Path) -> io::Result<()>{
    debug!("Entering {}", path.to_str().unwrap());
    for p in fs::read_dir(path)?{
        let e = p.unwrap();
        debug!("Found file {:?}", e.file_name());
        if e.file_type().unwrap().is_dir(){
            recursive_find(&e.path())?;
        }
        if e.file_type().unwrap().is_file(){
            let path = e.path();
            if path.extension().unwrap() == "coffee"{
               process_coffee(&path)?;
            }
        }
    }
    Ok(())
}

fn handle_event(event: DebouncedEvent, invalidation_tx: &Sender<InvalidatedPath>){
    use self::DebouncedEvent::*;
    let check = |p:&PathBuf|
        if p.extension().unwrap() == "coffee"{
            match process_coffee(&p){
                Ok(..) => {},
                Err(e) => debug!("Failed to process: {:?}", e)
            }
        };

    match event{
        Create(p) |
        Write(p)  => {
            let s = String::from(p.to_str().unwrap());
            debug!("File {} modified", s);
            check(&p);
            invalidation_tx.send(s).unwrap();
        },
        Rename(old, new) => {
            let s1 = String::from(old.to_str().unwrap());
            let s2 = String::from(new.to_str().unwrap());
            debug!("File {} renamed to {}", s1, s2);
            invalidation_tx.send(s1).unwrap();
            check(&new);
            invalidation_tx.send(s2).unwrap();
        },
        Remove(p) => {
            let s = String::from(p.to_str().unwrap());
            debug!("File {} removed", s);
            invalidation_tx.send(s).unwrap();
        }
        _ => ()
    }
}

pub fn launch_thread() -> (JoinHandle<()>, InvalidatedReceiver){
    let (invalidation_tx, invalidation_rx) = channel();
    
    let handle = thread::Builder::new()
        .name("rebuilder".into())
        .spawn(move ||{
        let watch_path = "client/";
        debug!("Finding and processing existing .coffee files");
        recursive_find(Path::new(watch_path)).unwrap();

        let (watcher_tx, watcher_rx) = channel();

        let mut watcher = watcher(watcher_tx, Duration::from_secs(5)).unwrap();

        watcher.watch(watch_path, RecursiveMode::Recursive).unwrap();

        loop{
            match watcher_rx.recv(){
                Ok(ev) => handle_event(ev, &invalidation_tx),
                Err(e) => debug!("watch error: {:?}", e)
            }
        }
    }).unwrap();

    (handle, invalidation_rx)
}
