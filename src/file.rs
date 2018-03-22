// guided by https://github.com/stephank/hyper-staticfile/blob/554215012b589288750406362527b6e94d5464b7/src/requested_path.rs

use hyper::server::{Request, Response, Service};
use hyper::Error;
use futures::{Future, future};

use std::path::{PathBuf};
use std::ops::Deref;
use std::rc::Rc;
use std::net::SocketAddr;
use std::sync::Arc;
use std::io;
use std::borrow::Borrow;

use filecache::FileCache;
use filethread::Response as FileResponse;

pub struct FileServerInternal{
    root:  PathBuf,
    cache: FileCache,
}

impl FileServerInternal{
    fn decode_path(&self, req: &Request) -> PathBuf{
        use std::str::FromStr;
        use std::path::Component;
        use regex::{Regex, Replacer, Captures};

        lazy_static!{
            static ref PERCENT_RE: Regex =
                Regex::new("%([0-9A-Fa-f]{2})").unwrap();
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
            PERCENT_RE.replace_all(req.path(), PercentReplacer))
        );
       
        // strip parent dir ("..") from the relative path before adding to base
        let root = {
            if self.root.is_relative(){
                ::std::env::current_dir().unwrap()
                                         .join(self.root.as_path())
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

fn io_error(io: io::Error, path: &String, reqpath: &String, reqaddr: &String) -> Response{
    use hyper::{StatusCode, Body};
    use std::io::ErrorKind::*;
    match io.kind(){
        PermissionDenied => {
            error!("{:>20} - 403 - {}", reqaddr, path);
            Response::new()
               .with_status(StatusCode::Forbidden)
               .with_body(Body::from(format!("<h1>HTTP 403 - Forbidden</h1><p>File <tt>\"{}\"</tt> forbidden</p>", reqpath)))
        },
        NotFound => {
            error!("{:>20} - 404 - {}", reqaddr, path);
            Response::new()
               .with_status(StatusCode::NotFound)
               .with_body(Body::from(format!("<h1>HTTP 400 - Not Found</h1><p>File <tt>\"{}\"</tt> not found</p>", reqpath)))
        },
        _ => {
            error!("{:>20} - Error: {:?}", reqaddr, io);
            Response::new()
               .with_status(StatusCode::InternalServerError)
               .with_body(Body::from(format!("<h1>HTTP 500 - Internal Server Error</h1><tt>{:?}</tt>", io)))
        }
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
        use std::str::FromStr;

        let method  = req.method().clone();
        let uri     = req.uri();
        let reqpath = Rc::new(String::from(req.path()));
        let reqaddr = Rc::new(format!("{}", req.remote_addr().unwrap_or(SocketAddr::from_str(&"0.0.0.0:0").unwrap())));
        if (method != Method::Head &&
            method != Method::Get) ||
            uri.is_absolute()
        {
            return box future::ok(Response::new().with_status(StatusCode::BadRequest))
        }
        
        let path = Arc::new(self.decode_path(&req));

        let fetch =
            self.cache
                .fetch(path.clone());
        let to_str = |p: Arc<PathBuf>| -> String {
            let b: &PathBuf = p.borrow();
            b.to_str()
             .unwrap_or("<nonunicode>".into())
             .into()
        };
        let path_str = to_str(path);
        box fetch
            .then(|r: Result<FileResponse,_>| r.unwrap())
            .then(move |r: FileResponse| Ok(
                  match r{
                      Err(io) => io_error(io, &path_str, reqpath.borrow(), reqaddr.borrow()),
                      Ok(smf) => {
                          // ToDo: etag?
                          let (mod_time, ref file) = *smf;
                          let size = file.len() as u64;
                          let modified = header::HttpDate::from(mod_time);
                          let mut res = Response::new()
                              .with_header(header::ContentLength(size))
                              .with_header(header::LastModified(modified));
                          
                          if method == Method::Get {
                              res.set_body(Body::from(file.clone()));
                          }
                          info!("{:>20} - 200 - {}", reqaddr, path_str);
                          res
                    }
                }))
        }
}

#[derive(Clone)]
pub struct FileServer(Rc<FileServerInternal>);

impl FileServer{
    pub fn new(root: PathBuf, cache: FileCache)
        -> FileServer
{
        FileServer(Rc::new(
            FileServerInternal{
                root,
                cache,
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
