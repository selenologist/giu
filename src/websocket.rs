use serde::{Deserialize, Serialize};
use rmp_serde;
use rmp_serde::{Deserializer, Serializer};
use ws::{listen, Handler, Factory, Sender, Handshake, Request, Response, Message, CloseCode};
use ws::{Error as WsError, ErrorKind as WsErrorKind, Result as WsResult};
use futures::future::{Future, IntoFuture};

use graph::{Response as GraphResponse, *};

use std::thread;
use std::thread::{JoinHandle};

#[derive(Copy,Clone)]
struct ReadyClient{
    graph: GraphId
}

impl ReadyClient{
    fn perform_command(&self, out: &Sender, store: &GraphStore,
                       command: Command) -> WsResult<()> {
        use graph::Command::*; 
        match command{
            AddLink((from_node, from_port), (to_node, to_port)) => {
                let graph = store.get(self.graph).unwrap();
                let mut graph_mut = graph.borrow_mut();

                let response = graph_mut.add_link(from_node, from_port, to_node, to_port);
                out.send(rmp_serde::encode::to_vec_named(&response).unwrap())
            },
            _ => {unimplemented!();}
        }
    }

    fn decode_command(msg: Message) -> WsResult<Command>{
        match msg{
            Message::Text(..) =>
                Err(WsError::new(
                    WsErrorKind::Protocol,
                    String::from("text received where binary msgpack expected"))),
            Message::Binary(b) => {
                let res: Result<Command, _> = rmp_serde::decode::from_slice(&b[..]);
                match res{
                    Ok(k)  => Ok(k),
                    Err(e) => Err(WsError::new(WsErrorKind::Protocol, format!("{:?}", e)))
                }
           }
       }
    }

    fn process_command(&self, out: &Sender, store: &GraphStore, msg: Message)
        -> WsResult<()>
    {
        match ReadyClient::decode_command(msg){
            Ok(k)  => self.perform_command(out, store, k),
            Err(e) => {error!("Decode_command failed: {:?}", e); Err(e)}
        }
    }
}

#[derive(Copy,Clone)]
enum ClientState{
    Ready(ReadyClient),
    AwaitingGraph
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct GraphChoice{
    id: GraphId
}

impl GraphChoice{
    fn decode(msg: Message) -> Result<GraphId, String>{
       match msg{
           Message::Text(..) => Err(String::from("Websocket protocol is msgpack therefore binary; Text received instead")),
           Message::Binary(b) => {
                let res: Result<GraphChoice, _> = rmp_serde::decode::from_slice(&b[..]);
                match res{
                    Ok(k) => Ok(k.id),
                    Err(e) => Err(format!("{:?}", e))
                }
           }
       }
    }
}

struct ServerHandler{
    out:   Sender,
    store: GraphStore,
    state: ClientState,
    addr:  String
}

impl Handler for ServerHandler{
    fn on_open(&mut self, hs: Handshake) -> WsResult<()>{
        if let Some(ip_addr) = hs.peer_addr {
            let ip_string = format!("{}", ip_addr);
            info!("{:>20} - connection established", ip_string);
            self.addr = ip_string;
        }
        else{
            debug!("Connection without IP address?");
        }


        self.out.send(rmp_serde::encode::to_vec_named(
            &GraphList::from_graphstore(&self.store))
            .unwrap())?;

        Ok(())
    }

    fn on_request(&mut self, req: &Request) -> WsResult<Response> {
        let mut res = Response::from_request(req)?;

        let protocol_name = "selenologist-node-editor";
        res.set_protocol(protocol_name);

        Ok(res)
    }
        
    fn on_message(&mut self, msg: Message) -> WsResult<()> {
        use self::ClientState::*;
        match self.state.clone() {
            Ready(client) =>
                client.process_command(&self.out, &self.store, msg),
            AwaitingGraph =>
                match GraphChoice::decode(msg){
                    Ok(id) => {
                        if self.store.contains_key(id){
                            self.state =
                                ClientState::Ready(
                                    ReadyClient{ graph: id });
                            Ok(())
                        }
                        else{
                            self.out.close(CloseCode::Unsupported)?;
                            Err(WsError::new(WsErrorKind::Protocol,
                                             "invalid graph ID"))
                        }
                    },
                    Err(e) => {
                        error!("{:?}", e);
                        self.out.close(CloseCode::Protocol)?;
                        Err(WsError::new(WsErrorKind::Protocol,
                                         "expected graph choice, got something else"))
                    }
                }
        }
    }
}

#[derive(Default)]
struct ServerFactory{
    store: GraphStore,    
}

impl Factory for ServerFactory{
    type Handler = ServerHandler;

    fn connection_made(&mut self, out: Sender) -> Self::Handler{
        ServerHandler{
            out,
            store: self.store.clone(),
            state: ClientState::AwaitingGraph,
            addr:  "0.0.0.0:0".into()
        }
    }
}

pub fn launch_thread() -> JoinHandle<()>{
    thread::Builder::new()
        .name("websocket".into())
        .spawn(move || {
        let mut factory = ServerFactory::default();
        let listen_addr = "127.0.0.1:3001";
        info!("Attempting to listen on {}", listen_addr);
        listen(listen_addr, |out| factory.connection_made(out)).unwrap()
    }).unwrap()
}
