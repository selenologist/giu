use hyper::header::ContentLength;
use hyper::server::{Http, Request, Response, Service, NewService};
use hyper::Error;
//use websocket::Websocket;
//use ws::async::server::upgrade::IntoWs;
//use ws::server::upgrade::HyperIntoWsError;
use tokio::executor::current_thread;
use tokio::net::{TcpListener};
use futures::{Future, Stream, future};

use std::path::Path;
use std::thread;
use std::thread::{JoinHandle};
use std::io;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use rebuilder::InvalidatedReceiver;
use file::FileServer;

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
        self.file.call(req)
    }
}

struct ServiceFactory{
    proto: MainService
}

impl ServiceFactory{
    fn new(file_threads: usize, invalidated_rx: InvalidatedReceiver) -> ServiceFactory{
        ServiceFactory {
            proto:
                MainService{
                    file: FileServer::new(file_threads, Path::new("client/"), invalidated_rx),
                    //wss_factory: websocket::ServerFactory::default()
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

pub fn launch_thread(invalidated_rx: InvalidatedReceiver) -> JoinHandle<()>{
    thread::Builder::new()
        .name("HTTP".into())
        .spawn(move ||{
    let addr_string = "127.0.0.1:3000";
    let addr        = addr_string.parse().unwrap();
    let factory     = ServiceFactory::new(4, invalidated_rx);
    let server      = Http::new().bind(&addr, factory).unwrap();

    info!("Starting server on http://{}", addr_string);
    server.run().unwrap();
    }).unwrap()
}
