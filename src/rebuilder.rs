use notify::{DebouncedEvent, Watcher, RecursiveMode, watcher};
use subprocess::{Exec, ExitStatus};

use futures::sync::mpsc::{channel as bounded_channel,
                          Sender as BoundedSender,
                          Receiver as BoundedReceiver,
                          SendError as BoundedSendError};
use futures::{future, future::Either, Future, Stream, Sink, Poll, Async};
use std::sync::mpsc::{channel as std_channel};
use std::time::Duration;
use std::io;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::thread::{JoinHandle};
use std::sync::Arc;

pub type InvalidationPath     = Arc<PathBuf>;
pub type InvalidationReceiver = BoundedReceiver<InvalidationPath>;
pub type InvalidationSender   = BoundedSender<InvalidationPath>;
type BoundedRecvError = ();
pub type InvalidationSendError = BoundedSendError<InvalidationPath>;
pub type InvalidationRecvError = BoundedRecvError;

static INVALIDATION_CHANNEL_SIZE: usize = 16;

pub struct InvalidationReceiverChain{
    invalidation_rx: InvalidationReceiver,
    daisy_tx: Option<InvalidationSender>
}

// repeats T along the daisy chain after the first receiver is done doing whatever it needs to do
// provides access to &T via deref and get
// first receiver must call .do_repeat() when ready for repeat
#[must_use]
pub struct RepeatAfter<T>(T, Option<BoundedSender<T>>);

impl<T: ::std::fmt::Debug> RepeatAfter<T>{
    pub fn get(&self) -> &T{
        &self.0
    }
    pub fn get_mut(&mut self) -> &mut T{
        &mut self.0
    }
    pub fn repeat(self) -> impl Future<Item=Option<BoundedSender<T>>, Error=BoundedSendError<T>>{
        if let Some(tx) = self.1{
            trace!("repeatafter called {:?}", self.0);
            Either::A(tx.send(self.0).map(|s| Some(s)))
        }
        else{
            Either::B(future::ok(None))
        }
    }
    pub fn repeat_block(self) -> Result<Option<BoundedSender<T>>, BoundedSendError<T>>{
        if let Some(tx) = self.1{
            tx.send(self.0).wait().map(|s| Some(s))
        }
        else{
            Ok(None)
        }
    }

}

impl<T: ::std::fmt::Debug> ::std::ops::Deref for RepeatAfter<T>{
    type Target = T;
    fn deref(&self) -> &Self::Target{
        self.get()
    }
}

impl InvalidationReceiverChain{
    pub fn with_daisy(invalidation_rx: InvalidationReceiver)
        -> (InvalidationReceiverChain, InvalidationReceiver)
    {
        let (daisy_tx, daisy_rx) = bounded_channel(INVALIDATION_CHANNEL_SIZE);
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
    pub fn try_recv(&mut self)  // repeat before returning value
        -> Poll<impl Future<Item=InvalidationPath, Error=InvalidationSendError>, InvalidationRecvError>
    {
        let p: InvalidationPath = match self.invalidation_rx.poll()?{
            Async::Ready(Some(path)) => path,
            _ => return Ok(Async::NotReady)
        };
        if let Some(tx) = self.daisy_tx.clone() {
            Ok(Async::Ready(
                    Either::A(tx.send(p.clone()).map(move |_| p))))
        }
        else{
            Ok(Async::Ready(
                    Either::B(future::ok(p))))
        }
    }
    pub fn recv(self)
        -> impl Stream<Item=Result<InvalidationPath, InvalidationSendError>, Error=InvalidationRecvError>
    { // repeat before returning value
        let daisy_tx = self.daisy_tx;
        self.invalidation_rx
            .map(move |p: InvalidationPath|
                 if let Some(tx) = daisy_tx.clone() {
                     tx.send(p.clone())
                       .map(move |_| p) // attach path to successful sends
                       .wait()
                 }
                 else{
                     Ok(p)
                 }
            )
    }
    pub fn recv_repeat_after(self) // repeat after RepeatAfter is dropped
        -> impl Stream<Item=RepeatAfter<InvalidationPath>, Error=InvalidationRecvError>{
        let (rx, daisy_tx) = (self.invalidation_rx, self.daisy_tx);
        rx.map(move |p: InvalidationPath|
               RepeatAfter(p, daisy_tx.clone()))
    }
    pub fn try_recv_repeat_after(&mut self) // repeat after RepeatAfter is dropped
        -> Poll<RepeatAfter<InvalidationPath>, InvalidationRecvError>{
        let p = match self.invalidation_rx.poll()?{
            Async::Ready(Some(path)) => path,
            _ => return Ok(Async::NotReady)
        };
        Ok(Async::Ready(RepeatAfter(p, self.daisy_tx.clone())))
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

fn handle_event(event: DebouncedEvent, invalidation_tx: &mut InvalidationSender){
    use self::DebouncedEvent::*;

    let broadcast = move |s| invalidation_tx.clone().send(Arc::new(s)).wait().unwrap();
    fn to_str<'a>(p: &'a PathBuf) -> &'a str{
        p.to_str().unwrap_or("<nonunicode>")
    }
    match event{
        Create(p) |
        Write(p)  => {
            info!("File {} modified", to_str(&p));
            check(&p);
            broadcast(p);
        },
        Rename(old, new) => {
            info!("File {} renamed to {}", to_str(&old), to_str(&new));
            broadcast(old);
            check(&new);
            broadcast(new);
        },
        Remove(p) => {
            info!("File {} removed", to_str(&p));
            broadcast(p);
        }
        _ => ()
    }
}

pub fn launch_thread() -> (JoinHandle<()>, InvalidationReceiver){
    // must be Arc<Mutex<Bus>> so that InvalidationReceiverMaker can add more receivers
    let (mut invalidation_tx, invalidation_rx) = bounded_channel(INVALIDATION_CHANNEL_SIZE);
    
    let handle = thread::Builder::new()
        .name("rebuilder".into())
        .spawn(move ||{
        let watch_path = "client/";
        trace!("Finding and processing existing .coffee files");
        recursive_find(Path::new(watch_path)).unwrap();

        let (watcher_tx, watcher_rx) = std_channel();

        let mut watcher = watcher(watcher_tx, Duration::from_millis(200)).unwrap();

        watcher.watch(watch_path, RecursiveMode::Recursive).unwrap();

        loop{
            match watcher_rx.recv(){
                Ok(ev) => handle_event(ev, &mut invalidation_tx),
                Err(e) => error!("watch error: {:?}", e)
            }
        }
    }).unwrap();

    (handle, invalidation_rx)
}
