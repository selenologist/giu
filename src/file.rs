// guided by https://github.com/stephank/hyper-staticfile/blob/554215012b589288750406362527b6e94d5464b7/src/requested_path.rs

use hyper::server::{Request, Response, Service};
use hyper::Error;
use futures::{Future, future, Sink, Stream};
use futures::future::Either;
use futures::sync::mpsc::channel as bounded_channel; // rename this because defaulting to bounded is dumb
use futures::sync::mpsc::Sender as BoundedSender;
use futures::sync::mpsc::Receiver as BoundedReceiver;
use tokio::executor::current_thread;

use std::collections::HashMap;
use std::thread;
use std::thread::{JoinHandle};
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::fs::File;
use std::time::SystemTime;
use std::vec::Vec;
use std::cell::{RefCell, Cell};
use std::ops::Deref;
use std::rc::Rc;

use rebuilder::InvalidatedReceiver;

type InMemoryFile = (SystemTime, Vec<u8>);
type ChannelPath = PathBuf;
type ChannelFile = io::Result<InMemoryFile>;

#[derive(Clone)]
pub struct FileCache(Rc<RefCell<(HashMap<String, Rc<InMemoryFile>>, InvalidatedReceiver)>>);

impl FileCache{
    pub fn new(invalidated_rx: InvalidatedReceiver) -> FileCache{
        FileCache(Rc::new(RefCell::new((HashMap::new(), invalidated_rx))))
    }

    pub fn try_invalidate(&self){
        use std::sync::mpsc::TryRecvError::Empty;
        let mut b = self.0.borrow_mut();
        loop{
            match b.1.try_recv(){
                Ok(s)  => {
                    debug!("Removing invalidated file {} from cache", s);
                    b.0.remove(&s); // we don't care if the key actually existed, so long as it's gone
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

    pub fn get(&self, key: &String) -> Option<Rc<InMemoryFile>>{
        self.try_invalidate();

        if let Some(v) = self.0.borrow().0.get(key){
            Some(v.clone())
        }
        else{
            None
        }
    }
    pub fn insert(&self, key: String, value: Rc<InMemoryFile>){
        self.0.borrow_mut().0.insert(key, value);
    }
}

struct FileServerThreadState{
    path_in:  BoundedReceiver<ChannelPath>,
    file_out: BoundedSender<ChannelFile>
}

struct FileServerThread{
    pub path_out: BoundedSender<ChannelPath>,
    pub file_in:  Rc<Cell<Option<BoundedReceiver<ChannelFile>>>>, // into_future() takes ownership but later returns. Swap the option in and out.
    pub handle:   JoinHandle<()>
}

impl FileServerThreadState{
    fn get_file(path: &Path) -> ChannelFile{
        debug!("fetching {}", path.to_str().unwrap());
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        let len      = metadata.len() as usize; // on 32bit systems we wouldn't be able to load >2^32 sized files anyway
        let mod_date = metadata.modified()?;
        let mut buf  = Vec::with_capacity(len); // XXX: this will explode on huge files
        let read     = file.read_to_end(&mut buf)?;
        assert!(read == len);
        debug!("read file {}", path.to_str().unwrap());

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
        debug!("exiting");
    }
}

impl FileServerThread{
    pub fn new(thread_number: usize) -> FileServerThread{
        // bounded_channel ensures that each thread has only one outstanding request at a time.
        let (path_out, path_in) = bounded_channel(1);
        let (file_out, file_in) = bounded_channel(1);
        let handle = thread::Builder::new()
            .name(format!("FileServer IO {}", thread_number).into())
            .spawn(move ||{
            let state = FileServerThreadState{
                path_in,
                file_out
            };
            state.run();
        }).unwrap();
        
        FileServerThread{
            file_in: Rc::new(Cell::new(Some(file_in))),
            path_out,
            handle
        }
    }
}

pub struct FileServerInternal{
    root:           PathBuf,
    cache:          FileCache,
    threads:        Vec<FileServerThread>,
    next_thread:    Cell<usize>, // used for round-robin selection of file threads
}

impl FileServerInternal{
    fn decode_path(&self, req: &Request) -> PathBuf{
        use std::str::FromStr;
        use std::path::Component;
        use regex::{Regex, Replacer, Captures};

        lazy_static!{
            static ref percent_re: Regex = Regex::new("%([0-9A-Fa-f]{2})").unwrap();
        }

        struct PercentReplacer;
        impl Replacer for PercentReplacer{
            // XXX kinda hacky
            fn replace_append(&mut self, caps: &Captures, dst: &mut String){
                #[allow(non_snake_case)]
                let nybble = |digit| {
                    // regex character class makes sure the character is definitely [0-9A-Fa-f]
                    let zero   = '0' as u32; // chars are always 4 bytes
                    let nine   = '9' as u32;
                    let upperA = 'A' as u32;
                    let upperF = 'F' as u32;
                    let lowerA = 'a' as u32;

                    if digit >= zero &&
                       digit <= nine {
                        (digit - zero) as u8
                    }
                    else if digit >= upperA &&
                            digit <= upperF {
                        (0xA + (digit - upperA)) as u8
                    }
                    else{
                        (0xA + (digit - lowerA)) as u8
                    }
                };


                if let Some(hex) = caps.get(1){
                    let hex   = hex.as_str();
                    let upper = char::from_str(&hex[0..1]).unwrap() as u32;
                    let lower = char::from_str(&hex[1..2]).unwrap() as u32;
                    let byte  = (nybble(upper) << 4) |
                                 nybble(lower);
                    dst.push(char::from(byte));
                }
            }
        }
        
        let without_percent = PathBuf::from(String::from(
            percent_re.replace_all(req.path(), PercentReplacer))
        );
       
        // strip parent dir ("..") from the relative path before adding to base
        let root = {
            if self.root.is_relative(){
                ::std::env::current_dir().unwrap()
                                         .join(self.root.clone())
            }
            else{
                self.root.clone()
            }
        };
        root.join(
            without_percent.components().fold(PathBuf::new(),
            |mut out, c|
            match c{
                Component::Normal(x) => {
                    out.push(x);
                    out
                },
                Component::ParentDir => {
                    out.pop();
                    out
                },
                _ => out
            }))
    }
}

type ResponseFuture = Box<Future<Item=Response, Error=Error>>;
impl Service for FileServer{
    type Request  = Request;
    type Response = Response;
    type Error    = Error;
    type Future   = ResponseFuture;

    fn call(&self, req: Request) -> Self::Future {
        use hyper::{Method, StatusCode, Body, header};
        let method = req.method().clone();
        let uri    = req.uri();
        if (method != Method::Head &&
            method != Method::Get) ||
            uri.is_absolute()
        {
            return box future::ok(Response::new().with_status(StatusCode::BadRequest))
        }
        
        let path = self.decode_path(&req);
        let path_string = String::from(path.to_str().unwrap());

        let fetch = 
            if let Some(cached) = self.cache.get(&path_string) {
                debug!("cache hit for {}", path_string);
                Either::A(future::ok(cached.clone()))
            }
            else{ 
                // access the IO thread pool in a round-robin fashion
                let current_thread = self.next_thread.get();
                let next_thread = current_thread + 1;
                if next_thread >= self.threads.len(){
                    // if the next thread exceeds the number of threads, the next thread is zero
                    self.next_thread.set(0);
                }
                else{
                    // otherwise, it's next_thread
                    self.next_thread.set(next_thread);
                }

                let io = &self.threads[current_thread];
                let path_out = io.path_out.clone();
                let outer_file_in = io.file_in.clone();
                let file_in = outer_file_in.replace(None).unwrap();
                let self2 = self.clone(); 
                Either::B(
                    path_out.send(path)
                    .map_err(|_| panic!())
                    .and_then({let ps = path_string.clone(); move |_|{
                        debug!("awaiting IO thread response for {}", ps);
                        file_in.into_future()
                    }})
                    .map_err(|_| panic!())
                    .and_then(move |(to_cache, file_in)| {
                        outer_file_in.replace(Some(file_in));
                        let cf: ChannelFile = to_cache.expect("Io thread failure");
                        let imf: Rc<InMemoryFile> = match cf{
                            Ok(k) => Rc::new(k),
                            Err(e) => {return Err(e)}
                        };
                        debug!("storing {} in cache", path_string);
                        self2.cache.insert(path_string.clone(), imf.clone());
                        Ok(imf)
                    }))
            };
        
        box fetch.and_then(move |imf: Rc<InMemoryFile>| {
            // ToDo: etag?
            let (mod_time, file) = (&imf.0, imf.1.clone());
            let size = file.len() as u64;
            let modified = header::HttpDate::from(*mod_time);
            let mut res = Response::new()
                .with_header(header::ContentLength(size))
                .with_header(header::LastModified(modified));

            if method == Method::Get {
                res.set_body(Body::from(file));
            }
            Ok(res)
        }).or_else(|e| {
                debug!("Error: {:?}", e);
                Ok(Response::new().with_status(StatusCode::BadRequest))
            })
    }
}

#[derive(Clone)]
pub struct FileServer(Rc<FileServerInternal>);

impl FileServer{
    pub fn new(threads: usize, base_path: &Path, invalidated_rx: InvalidatedReceiver) -> FileServer{
        let threads = (0..threads).map(FileServerThread::new).collect();
        FileServer(Rc::new(
            FileServerInternal{
                root:           base_path.into(),
                cache:          FileCache::new(invalidated_rx),
                next_thread:    Cell::new(0),
                threads,
            }
        ))
    }
}

impl Deref for FileServer{
    type Target = FileServerInternal;
    fn deref(&self) -> &Self::Target{
        &self.0
    }
}
