use futures::{Future, Stream, future};
use hyper::header::ContentLength;
use hyper::server::{Http, Request, Response, Service};
use hyper::Error;
use hyper_staticfile::Static;
use tokio_core::reactor::{Core, Handle};
use tokio_core::net::TcpListener;
use std::path::Path;
use std::thread;
use std::thread::{JoinHandle};
use std::io;

type ResponseFuture = Box<Future<Item=Response, Error=Error>>;

struct MainService{
    static_files: Static
}

impl MainService{
    fn new(handle: &Handle) -> MainService{
        MainService {
            static_files: Static::new(handle, Path::new("./client/")),
        }
    }
}

impl Service for MainService {
    type Request = Request;
    type Response = Response;
    type Error = Error;
    type Future = ResponseFuture;

    fn call(&self, req: Request) -> Self::Future {
        use std::str::FromStr;
        if req.path() == "/" {
            self.static_files.call(::hyper::Request::new(::hyper::Method::Get, ::hyper::Uri::from_str("/index.html").unwrap()))
        }
        else{
            self.static_files.call(req)
        }
    }
}

pub fn launch_thread() -> JoinHandle<()>{
    thread::spawn(move ||{
        let mut core = Core::new().unwrap();
        let handle   = core.handle();

        let addr = "127.0.0.1:3000".parse().unwrap();
        let listener = TcpListener::bind(&addr, &handle).unwrap();

        let http = Http::new();
        let server = listener.incoming().for_each(|(sock, addr)| {
            let s = MainService::new(&handle);
            http.bind_connection(&handle, sock, addr, s);
            Ok(())
        });

        println!("[http] static server running on http://localhost:3000");
        core.run(server).unwrap();
    })
}
