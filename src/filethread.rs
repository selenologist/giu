use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::cell::Cell;
use std::rc::Rc;
use std::fs::File;
use std::thread;
use std::thread::JoinHandle;

use futures::{Future, future, Sink, Stream};
use futures::future::Either;
use futures::sync::mpsc::channel as bounded_channel; // rename this because defaulting to bounded is dumb
use futures::sync::mpsc::Sender as BoundedSender;
use futures::sync::mpsc::Receiver as BoundedReceiver;
use tokio::executor::current_thread;

use file::InMemoryFile;
use filecache::FileCache;

type ChannelPath = PathBuf;
type ChannelFile = io::Result<InMemoryFile>;
pub type FileThreadResult = Future<Item=Rc<InMemoryFile>, Error=(String, io::Error)>; // this is what the fetch futures are, but type aliases cannot actually be used as traits

struct FileThreadState{
    path_in:  BoundedReceiver<ChannelPath>,
    file_out: BoundedSender<ChannelFile>
}

pub struct FileThread{
    pub path_out: BoundedSender<ChannelPath>,
    pub file_in:  Rc<Cell<Option<BoundedReceiver<ChannelFile>>>>, // into_future() takes ownership but later returns. Swap the option in and out.
    pub handle:   JoinHandle<()>
}

impl FileThreadState{
    fn get_file(path: &Path) -> ChannelFile{
        trace!("Fetching {}", path.to_str().unwrap());
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        let len      = metadata.len() as usize; // on 32bit systems we wouldn't be able to load >2^32 sized files anyway
        let mod_date = metadata.modified()?;
        let mut buf  = Vec::with_capacity(len); // XXX: this will explode on huge files
        let read     = file.read_to_end(&mut buf)?;
        assert!(read == len);
        trace!("Read file {}", path.to_str().unwrap());

        Ok((mod_date, buf))
    }
    pub fn run(self){
        current_thread::run(move |_| {
            let path_in  = self.path_in;
            let file_out = self.file_out;
            let task = path_in.for_each(
                move |p| {
                    current_thread::spawn(
                        file_out.clone()
                            .send(Self::get_file(&p))
                            .map_err(|_| panic!())
                            .map(|_| ()));
                    Ok(())
                }
            );

            current_thread::spawn(task);
        });
    }
}

impl FileThread{
    pub fn new(thread_number: usize) -> FileThread{
        // bounded_channel ensures that each thread has only one outstanding request at a time.
        let (path_out, path_in) = bounded_channel(1);
        let (file_out, file_in) = bounded_channel(1);
        let handle = thread::Builder::new()
            .name(format!("File IO {}", thread_number).into())
            .spawn(move ||{
            let state = FileThreadState{
                path_in,
                file_out
            };
            state.run();
        }).unwrap();
        
        FileThread{
            file_in: Rc::new(Cell::new(Some(file_in))),
            path_out,
            handle
        }
    }
}

pub struct FileThreadPool{
    threads:        Vec<FileThread>,
    next_thread:    Cell<usize>, // used for round-robin selection of file threads
}

impl FileThreadPool{
    pub fn new(threads: usize) -> FileThreadPool {
        let threads = (0..threads).map(FileThread::new).collect();
        FileThreadPool{
            threads,
            next_thread: Cell::new(0)
        }
    }

    pub fn fetch(&self, path: PathBuf) -> impl Future<Item=Rc<InMemoryFile>, Error=(String, io::Error)>{ // FileThreadResult
        // access the IO thread pool in a round-robin fashion
        let current_thread = self.next_thread.get();
        let next_thread    = current_thread + 1;
        if next_thread >= self.threads.len(){
            // if the next thread exceeds the number of threads, the next thread is zero
            self.next_thread.set(0);
        }
        else{
            // otherwise, it's next_thread
            self.next_thread.set(next_thread);
        }

        let io              = &self.threads[current_thread];
        let path_out        = io.path_out.clone();
        let outer_file_in   = io.file_in.clone();
        let file_in         = outer_file_in.replace(None);
        let out_path        = path.clone();
        
        if let Some(file_in) = file_in{
            Either::A(path_out
                .send(out_path)
                .map_err(|_| panic!())
                .and_then(|_|{
                    file_in.into_future()
                })
                .map_err(|_| panic!())
                .and_then(move |(result, file_in)| {
                    outer_file_in.replace(Some(file_in));
                    let cf: ChannelFile = result.expect("Io thread failure");
                    match cf{
                        Ok(k)  => Ok(Rc::new(k)),
                        Err(e) => Err((format!("{}", path.display()), e))
                    }
                }))
        }
        else{
            Either::B(
                future::err(
                    (format!("{}", path.display()),
                    io::Error::new(io::ErrorKind::WouldBlock, "Thread is busy")) // XXX handle this properly (will do for now)
                )
            )
        }
    }

    pub fn fetch_with_cache(&self, cache: FileCache, path: PathBuf) -> impl Future<Item=Rc<InMemoryFile>, Error=(String, io::Error)>{
        let path_string = format!("{}", path.display());
        if let Some(imf) = cache.get(&path_string) {
            Either::A(future::ok(imf)) // was cached, return immediately
        }
        else{
            Either::B(self
                .fetch(path)
                .and_then(move |imf| {
                    cache.insert(path_string, imf.clone());
                    Ok(imf)
                }))
        }
    }
}

