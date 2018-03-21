use std::sync::Arc;
use std::collections::HashMap;
use std::thread;
use std::thread::{JoinHandle};
use std::borrow::Borrow;
use std::io;

use futures::{future, Future};
use futures::sync::mpsc::channel as bounded_channel;
use futures::sync::mpsc::Sender as BoundedSender;
use futures::sync::mpsc::Receiver as BoundedReceiver;
use futures::sync::oneshot::{Receiver as OneshotReceiver,
                             Sender as OneshotSender,
                             channel as oneshot_channel};

use rebuilder::InvalidationReceiverChain;
use filethread::{SharedMemoryFile, FileThreadPool, RequestPath};

type Request     = (RequestPath, OneshotSender<Response>);
type Response    = io::Result<SharedMemoryFile>;

#[derive(Clone)]
pub struct FileCache(Arc<FileCacheThread>);

impl FileCache{
    pub fn new(n_threads: usize,
               invalidation_chain: InvalidationReceiverChain) -> FileCache{
        let (req_out, req_in) = bounded_channel(n_threads); // enough channel space to fill all threads
        let handle = thread::Builder::new()
            .name("filecache".into())
            .spawn(move || {
                let state = FileCacheState::new(n_threads, invalidation_chain, req_in);
                state.run()
            });
        FileCache(Arc::new(
            FileCacheThread{
                req_out,
                handle
            }
        ))
    }
    pub fn fetch(&self, path: RequestPath)
        -> impl Future<Item  = SharedMemoryFile,
                       Error = io::Error>
    {
        self.0.fetch(path)
    }
}

struct FileCacheThread{
    req_out: BoundedSender<Request>,
    handle:  JoinHandle<()>
}

pub struct FileCacheState{
    req_in:             BoundedReceiver<Request>,
    store:              HashMap<RequestPath, SharedMemoryFile>,
    invalidation_chain: InvalidationReceiverChain,
    file_threads:       FileThreadPool
}

impl FileCacheState{
    pub fn new(n_threads: usize,
               invalidation_chain: InvalidationReceiverChain,
               req_in: BoundedReceiver<Request>) -> FileCacheState{
        FileCacheState{
            req_in,
            invalidation_chain,
            store: HashMap::new(),
            file_threads: FileThreadPool::new(n_threads)
        }
    }

    pub fn try_invalidate(&mut self){
        use std::sync::mpsc::TryRecvError::Empty;
        loop{
            match self.invalidation_chain
                      .try_recv_repeat_after(){
                Ok(a)  => {
                    let p = a.get();
                    trace!("Removing invalidated file {:?} from cache", p);
                    self.store.remove(p); // we don't care if the key actually existed, so long as it's gone
                },
                Err(e) => {
                    if e == Empty{
                        break; // no more messages, just exit
                    }
                    else{
                        panic!("Cache invalidation message receiver disconnected!");
                    }
                }
            }
        }
    }

    pub fn get(&mut self, key: Borrow<PathBuf>) -> Option<SharedMemoryFile>{
        self.try_invalidate();

        if let Some(v) = self.store.get(key){
            trace!("Cache hit {}", key);
            Some(v.clone())
        }
        else{
            trace!("Cache miss {}", key);
            None
        }
    }

    pub fn insert(&mut self, key: PathBuf, value: SharedMemoryFile){
        trace!("Storing {} in cache", key);
        self.insert(key, value); // we don't care if the file was already cached
    }

    pub fn run(mut self){
       loop{
            match self.invalidation_chain // XXXXXXX
       }
    }
}

