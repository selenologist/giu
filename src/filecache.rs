use std::sync::Arc;
use std::collections::HashMap;
use std::thread;
use std::thread::{JoinHandle};
use std::io;

use futures::{Future, Sink, Stream};
use futures::sync::mpsc::channel as bounded_channel;
use futures::sync::mpsc::Sender as BoundedSender;
use futures::sync::mpsc::Receiver as BoundedReceiver;
use futures::sync::oneshot::{Sender as OneshotSender,
                             channel as oneshot_channel,
                             Canceled};

use rebuilder::{InvalidationPath, InvalidationReceiverChain, RepeatAfter};
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
            }).unwrap();

        FileCache(Arc::new(
            FileCacheThread{
                req_out,
                handle
            }
        ))
    }
    pub fn fetch(&self, path: RequestPath)
        -> impl Future<Item  = Response,
                       Error = Canceled>
    {
        self.0.fetch(path)
    }
}

struct FileCacheThread{
    req_out: BoundedSender<Request>,
    handle:  JoinHandle<()>
}

impl FileCacheThread{
    pub fn fetch(&self, path: RequestPath)
        -> impl Future<Item  = Response,
                       Error = Canceled>
    {
        let (resp_out, resp_in) = oneshot_channel();
        self.req_out
            .clone() // either I clone or I Mutex, I think this is better
            .send((path, resp_out))
            .map_err(|e| panic!(e))
            .and_then(|_| resp_in)
    }
}

type CacheStore = HashMap<RequestPath, SharedMemoryFile>;
pub struct FileCacheState{
    req_in:             BoundedReceiver<Request>,
    store:              CacheStore,
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

    pub fn invalidate(store: &mut CacheStore, path: &InvalidationPath){
       trace!("Removing invalidated file {:?}", path);
       store.remove(path);
    }

    pub fn get(store: &CacheStore, key: &RequestPath) -> Option<SharedMemoryFile>{
        if let Some(v) = store.get(key){
            trace!("Cache hit {}", key.to_str().unwrap_or("<nonunicode>"));
            Some(v.clone())
        }
        else{
            trace!("Cache miss {}", key.to_str().unwrap_or("<nonunicode>"));
            None
        }
    }

    pub fn insert(store: &mut CacheStore, key: RequestPath, value: SharedMemoryFile){
        trace!("Storing {} in cache", key.to_str().unwrap_or("<nonunicode>"));
        store.insert(key, value); // we don't care if the file was already cached
    }

    pub fn run(mut self){
        // I know. This is awful. I did whatever I could to make it compile.

        let (mut store, file, chain, req_in) =
            (&mut self.store, self.file_threads, self.invalidation_chain, self.req_in);

        enum FS<F,S>{
            First(F),
            Second(S)
        };

        let first = |a: RepeatAfter<InvalidationPath>, s: &mut CacheStore|{
            Self::invalidate(s, a.get());
            a.do_repeat_block().unwrap();
        };

        let second = |req: Request, s: &mut CacheStore|{
            let (path, resp_out) = req;
            let (cached, mut next_store) = {
                let osmf = {
                    Self::get(&s, &path)
                };
                (osmf, s)
            };
            if let Some(hit) = cached{
                resp_out.send(Ok(hit))
                        .unwrap();
            }
            else{
                let s = &mut next_store;
                file.fetch(path.clone())
                    .map_err(|e| panic!(e))
                    .map(|r|{
                        if let Ok(ref smf) = r{
                            Self::insert(s, path, smf.clone());
                        }
                        resp_out.send(r).unwrap();
                     })
                    .wait().unwrap()
            }
        };


        chain
            .recv_repeat_after()
            .map(|f| FS::First(f)) 
            .map_err(|_| ())
            .select(req_in.map(|s| FS::Second(s))
                          .map_err(|_| ()))
            .map(|r| { match r{
                FS::First(a)    => first(a, &mut store),
                FS::Second(req) => second(req, &mut store)
            }
        }).wait()
          .last()
          .unwrap()
          .unwrap();
    }
}

