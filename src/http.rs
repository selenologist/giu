use hyper::server::{Http, Request, Response, Service, NewService};
use hyper::Error;
use futures::Future;

use std::thread;
use std::thread::{JoinHandle};
use std::io;
use time;

use file::FileServer;
use filecache::FileCache;

type ResponseFuture = Box<Future<Item=Response, Error=Error>>;

#[derive(Clone)]
struct MainService{
    file: FileServer
}

impl Service for MainService {
    type Request  = Request;
    type Response = Response;
    type Error    = Error;
    type Future   = ResponseFuture;

    fn call(&self, req: Request) -> Self::Future {
        profile!(format!("{}", req.path()), {self.file.call(req)})
    }
}

struct ServiceFactory{
    proto: MainService
}

impl ServiceFactory{
    fn new(cache: FileCache)
        -> ServiceFactory {
        ServiceFactory {
            proto:
                MainService{
                    file: FileServer::new("client/".into(), cache),
                }
        }
    }
}

impl NewService for ServiceFactory{
    type Request = Request;
    type Response = Response;
    type Error = Error;
    type Instance = MainService;

    fn new_service(&self) -> Result<Self::Instance, io::Error>{
        Ok(self.proto.clone())
    }
}

pub fn launch_thread(cache: FileCache)
    -> JoinHandle<()>{
    thread::Builder::new()
        .name("HTTP".into())
        .spawn(move ||{
    let addr_string = "127.0.0.1:3000";
    let addr        = addr_string.parse().unwrap();
    let factory     = ServiceFactory::new(cache);
    let server      = Http::new().bind(&addr, factory).unwrap();

    info!("Starting server on http://{}", addr_string);
    server.run().unwrap();
    }).unwrap()
}
