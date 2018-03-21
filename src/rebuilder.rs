use notify::{DebouncedEvent, Watcher, RecursiveMode, watcher};
use subprocess::{Exec, ExitStatus};

use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError, RecvError};
use std::time::Duration;
use std::io;
use std::fs;
use std::path::{Path};
use std::thread;
use std::thread::{JoinHandle};
use std::mem::replace;

pub type InvalidationPath = String;
pub type InvalidationReceiver = Receiver<InvalidationPath>;
pub type InvalidationSender = Sender<InvalidationPath>;

pub struct InvalidationReceiverChain{
    invalidation_rx: InvalidationReceiver,
    daisy_tx: Option<InvalidationSender>
}

// repeats T on a hidden Sender when it goes out of scope;
// provides access to &T via deref and get
// Option<T> so we can move the original value out on Drop
pub struct RepeatAfter<T>(Option<T>, Option<Sender<T>>);

impl<T> RepeatAfter<T>{
    pub fn get(&self) -> &T{
        self.0.as_ref().unwrap()
    }
    pub fn get_mut(&mut self) -> &mut T{
        self.0.as_mut().unwrap()
    }
}

impl<T> ::std::ops::Drop for RepeatAfter<T>{
    fn drop(&mut self){
        if let Some(ref tx) = self.1{
            tx.send(replace(&mut self.0, None).unwrap()).unwrap();
        }
    }
}

impl<T> ::std::ops::Deref for RepeatAfter<T>{
    type Target = T;
    fn deref(&self) -> &Self::Target{
        self.get()
    }
}

impl InvalidationReceiverChain{
    pub fn with_daisy(invalidation_rx: InvalidationReceiver)
        -> (InvalidationReceiverChain, InvalidationReceiver)
    {
        let (daisy_tx, daisy_rx) = channel();
        (InvalidationReceiverChain{
            invalidation_rx,
            daisy_tx: Some(daisy_tx)
        },
         daisy_rx)
    }
    pub fn without_daisy(invalidation_rx: InvalidationReceiver)
        -> InvalidationReceiverChain
    {
        InvalidationReceiverChain{
            invalidation_rx,
            daisy_tx: None
        }
    }
    pub fn try_recv(&self)  // immediate repeat
        -> Result<InvalidationPath, TryRecvError>
    {
        let p = self.invalidation_rx.try_recv()?;
        if let Some(ref tx) = self.daisy_tx {
            tx.send(p.clone()).unwrap();
        }
        Ok(p)
    }
    pub fn recv(&self) -> Result<InvalidationPath, RecvError>{ // immediate repeat
        let p = self.invalidation_rx.recv()?;
        if let Some(ref tx) = self.daisy_tx {
            tx.send(p.clone()).unwrap();
        }
        Ok(p)
    }
    pub fn recv_repeat_after(&self) // repeat after RepeatAfter is dropped
        -> Result<RepeatAfter<InvalidationPath>, RecvError>{
        let p = self.invalidation_rx.recv()?;
        Ok(RepeatAfter(Some(p), self.daisy_tx.clone()))
    }
    pub fn try_recv_repeat_after(&self) // repeat after RepeatAfter is dropped
        -> Result<RepeatAfter<InvalidationPath>, TryRecvError>{
        let p = self.invalidation_rx.try_recv()?;
        Ok(RepeatAfter(Some(p), self.daisy_tx.clone()))
    }
}

impl Into<InvalidationReceiverChain> for InvalidationReceiver{
    fn into(self) -> InvalidationReceiverChain{
        InvalidationReceiverChain::without_daisy(self)
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
        trace!("Found file {:?}", e.file_name());
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

fn handle_event(event: DebouncedEvent, invalidation_tx: &InvalidationSender){
    use self::DebouncedEvent::*;

    let broadcast = |s| invalidation_tx.send(s).unwrap();
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

pub fn launch_thread() -> (JoinHandle<()>, InvalidationReceiver){
    // must be Arc<Mutex<Bus>> so that InvalidationReceiverMaker can add more receivers
    let (invalidation_tx, invalidation_rx) = channel();
    
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

    (handle, invalidation_rx)
}
