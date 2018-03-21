use ws::{Handler, Factory, Sender, Handshake, Request, Response as WsResponse, CloseCode, WebSocket};
use ws::{Result as WsResult};

use std::thread;
use std::thread::{JoinHandle};
use std::sync::mpsc::RecvError;

use rebuilder::InvalidationReceiverChain;
use log::Level;

// does nothing but keep the connection open and keep address if trace is on
#[derive(Default)]
struct NullHandler{
    addr: Option<String>
}
struct ServerFactory; // builds NullHandlers

impl Handler for NullHandler{
    fn on_open(&mut self, hs: Handshake) -> WsResult<()>{
        if log_enabled!(Level::Trace){ // don't bother populating addr if not Tracing
            if let Some(ip_addr) = hs.peer_addr {
                let ip_string = format!("{}", ip_addr);
                info!("{:>20} - connection established",
                      ip_string);
                self.addr = Some(ip_string);
            }
        }
        Ok(())
    }

    fn on_request(&mut self, req: &Request) -> WsResult<WsResponse> {
        let mut res = WsResponse::from_request(req)?;

        let protocol_name = "selenologist-minimal-reloader";
        res.set_protocol(protocol_name);

        Ok(res)
    }

    fn on_close(&mut self, code: CloseCode, reason: &str){
        trace!("Closing connection {:?} because {:?} {}", self.addr, code, reason);
    }
}

impl Factory for ServerFactory{
    type Handler = NullHandler;

    fn connection_made(&mut self, _out: Sender) -> Self::Handler{
        NullHandler::default()
    }
}

pub fn launch_thread(invalidation_rx: InvalidationReceiverChain)
    -> JoinHandle<()>
{
    thread::Builder::new()
        .name("reloader".into())
        .spawn(move || {
            let factory = ServerFactory;
            let listen_addr = "127.0.0.1:3002";
            info!("Attempting to listen on {}", listen_addr);
            let server = WebSocket::new(factory).unwrap();
            let broadcaster = server.broadcaster();
            // lazily spawn another thread to handle the mpsc events
            let handle = thread::Builder::new()
                .name("reloader bcast".into())
                .spawn(move || -> Result<(), RecvError>{
                    loop{
                        let p = invalidation_rx.recv()?;
                        trace!("got {}", p);
                        if p.ends_with("main.js"){
                            broadcaster.send("Reload").unwrap();
                        }
                    }
                }).unwrap();
            server.listen(listen_addr).unwrap();
            let join = handle.join();
            panic!("reloader broadcast thread joined: {:?}", join)
        }).unwrap()
}

