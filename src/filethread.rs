use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::cell::Cell;
use std::sync::Arc;
use std::fs::File;
use std::time::SystemTime;
use std::thread;
use std::thread::JoinHandle;

use futures::{Future, future, Sink, Stream};
use futures::future::Either;
use futures::sync::mpsc::channel as bounded_channel; // rename this because defaulting to bounded is dumb
use futures::sync::mpsc::Sender as BoundedSender;
use futures::sync::mpsc::Receiver as BoundedReceiver;
use futures::sync::oneshot::{Receiver as OneshotReceiver,
                             Sender as OneshotSender,
                             channel as oneshot_channel};

pub type InMemoryFile = (SystemTime, Vec<u8>);
pub type SharedMemoryFile = Arc<InMemoryFile>;

pub type RequestPath = PathBuf;
type Request     = (RequestPath, OneshotSender<Response>);
type Response    = io::Result<SharedMemoryFile>;
pub type FileThreadResult = OneshotReceiver<Response>; // this is what the fetch futures are, but type aliases cannot actually be used as traits

struct FileThreadState(BoundedReceiver<Request>);

impl FileThreadState{
    fn get_file(path: &Path) -> Response{
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
                resp.send(Self::get_file(&path))
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

pub struct FileThreadPool{
    threads:        Vec<FileThread>,
    next_thread:    Cell<usize>, // used for round-robin selection of file threads
}

impl FileThreadPool{
    pub fn new(n_threads: usize) -> FileThreadPool {
        let threads = (0..n_threads).map(FileThread::new).collect();
        FileThreadPool{
            threads,
            next_thread: Cell::new(0)
        }
    }

    pub fn fetch(&self, path: PathBuf)
        -> impl Future<Item=SharedMemoryFile, Error=io::Error> // basically Future<FileThreadResult>
    {
        // access the IO thread pool in a round-robin fashion
        let current_thread = self.next_thread.get();
        let next_thread =
            current_thread.checked_add(1) // usize would have overflowed to zero anyway
                          .unwrap_or(0); // but ensure it definitely safely overflows
        if next_thread >= self.threads.len(){
            // if the next thread exceeds the number of threads, the next thread is zero
            self.next_thread.set(0);
        }
        else{
            // otherwise, it's next_thread
            self.next_thread.set(next_thread);
        }

        let req_out = &self.threads[current_thread].req_out;
       
        let (file_out, file_in) = oneshot_channel();
        req_out
            .clone() // easier to clone than to &mut self
            .send((path, file_out))
            .map_err(|_| panic!())
            .map(move |_|{
                file_in
            })
    }
}

