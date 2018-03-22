use std::io;
use std::io::Read;
use std::path::{PathBuf};
use std::sync::{Arc, atomic::AtomicUsize};
use std::fs::File;
use std::time::SystemTime;
use std::thread;
use std::thread::JoinHandle;
use std::borrow::Borrow;

use futures::{Future, Sink, Stream};
use futures::sync::mpsc::channel as bounded_channel; // rename this because defaulting to bounded is dumb
use futures::sync::mpsc::Sender as BoundedSender;
use futures::sync::mpsc::Receiver as BoundedReceiver;
use futures::sync::oneshot::{Sender as OneshotSender,
                             channel as oneshot_channel,
                             Canceled};

pub type InMemoryFile = (SystemTime, Vec<u8>);
pub type SharedMemoryFile = Arc<InMemoryFile>;

pub type RequestPath = Arc<PathBuf>;
    type Request     = (RequestPath, OneshotSender<Response>);
pub type Response    = io::Result<SharedMemoryFile>;
// pub type FileThreadResult = OneshotReceiver<Response>; <-- File thread responses look like this

struct FileThreadState(BoundedReceiver<Request>);

impl FileThreadState{
    fn get_file(path: &PathBuf) -> Response{
        trace!("Fetching {}", path.to_str().unwrap());
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        let len      = metadata.len() as usize; // on 32bit systems we wouldn't be able to load >2^32 sized files anyway
        let mod_date = metadata.modified()?;
        let mut buf  = Vec::with_capacity(len); // XXX: this will explode on huge files
        let read     = file.read_to_end(&mut buf)?;
        assert!(read == len);
        trace!("Read file {}", path.to_str().unwrap());

        Ok(Arc::new((mod_date, buf)))
    }
    pub fn run(self){
        let task = self.0.for_each(
            move |req| {
                let (path, resp) = req;
                resp.send(Self::get_file(path.borrow()))
                    .unwrap();
                Ok(())
            }
        );

        task.wait().unwrap();
    }
}

impl FileThread{
    pub fn new(thread_number: usize) -> FileThread{
        // bounded_channel ensures that each thread has only one outstanding request at a time.
        let (req_out, req_in)   = bounded_channel(1);
        let handle = thread::Builder::new()
            .name(format!("File IO {}", thread_number).into())
            .spawn(move ||{
            let state = FileThreadState(req_in);
            state.run();
        }).unwrap();
        
        FileThread{
            req_out,
            handle
        }
    }
}

pub struct FileThread{
    pub req_out: BoundedSender<Request>,
    pub handle:  JoinHandle<()>
}

pub struct RoundRobin{
    counter: AtomicUsize,
    max:     usize
}

impl RoundRobin{
    pub fn new(max: usize) -> RoundRobin{
        RoundRobin{
            counter: AtomicUsize::new(0),
            max
        }
    }
    pub fn get_next(&self) -> usize{
        use std::sync::atomic::Ordering;
        // warning: will be discontinuous at 2^64 operations due to overflow.
        // If it ever gets that high.
        self.counter.fetch_add(1, Ordering::AcqRel) % self.max

        /* below implementation performs poorly when crowded
         * test tests::bench_crowded_roundrobin ... bench: 43,618 ns/iter (+/- 40,604)
         * test tests::bench_roundrobin         ... bench:     12 ns/iter (+/- 0) 
         * vs
         * test tests::bench_crowded_roundrobin ... bench: 2,908 ns/iter (+/- 3,542)
         * test tests::bench_roundrobin         ... bench:    11 ns/iter (+/- 0)
         * for the modulo version
        loop{
           let current = self.counter.load(Ordering::Acquire);
           let next    = {
               let next = current+1;
               if next < self.max{
                   next
               }
               else{
                   0
               }
           };
           let x= self.counter.compare_and_swap(current, next, Ordering::Release);
           if x == current{
               return current
           }
       }*/
    }
}

pub struct FileThreadPool{
    threads:   Vec<FileThread>,
    scheduler: RoundRobin
}

impl FileThreadPool{
    pub fn new(n_threads: usize) -> FileThreadPool {
        let threads = (0..n_threads).map(FileThread::new).collect();
        FileThreadPool{
            threads,
            scheduler: RoundRobin::new(n_threads)
        }
    }

    pub fn fetch(&self, path: RequestPath)
        -> impl Future<Item=Response, Error=Canceled>
    {
        // access the IO thread pool in a round-robin fashion
        let current_thread = self.scheduler.get_next();
        let req_out = &self.threads[current_thread].req_out;
       
        let (file_out, file_in) = oneshot_channel();
        req_out
            .clone() // easier to clone than to &mut self
            .send((path, file_out))
            .map_err(|_| panic!())
            .and_then(move |_|{
                file_in
            })
    }
}

