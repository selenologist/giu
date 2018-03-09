use serde::{Deserialize, Serialize};
use rmp_serde;
use rmp_serde::{Deserializer, Serializer};
use ws::OwnedMessage;
use ws::codec::ws::MessageCodec;
use ws::client::async::{ClientNew, Client, Framed, TcpStream};
use hyper::header::Headers; // XXX consider handling this in http.rs
use futures::future::{Future, IntoFuture};

use graph::*;

use std::cell::RefCell;
use std::thread;
use std::thread::{Thread, JoinHandle};
use std::io;
use std::net::SocketAddr;

#[derive(Copy,Clone)]
struct ReadyClient{
    graph: GraphId
}

impl ReadyClient{
    fn decode_command(msg: Message) -> Result<Command, _>{
        match msg{
            Message::Text(..) => Err(::ws::Error::new(::ws::ErrorKind::Protocol, String::from("Websocket protocol is msgpack therefore binary; Text received instead"))),
            Message::Binary(b) => {
                let res: Result<Command, _> = rmp_serde::decode::from_slice(&b[..]);
                match res{
                    Ok(k)  => Ok(k),
                    Err(e) => Err(::ws::Error::new(::ws::ErrorKind::Protocol, format!("{:?}", e)))
                }
           }
       }
    }

    fn perform_command(&self, out: &Sender, store: &GraphStore,
                       command: Command) -> Result<(), _> {
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

    fn process_command(&self, out: &Sender, store: &GraphStore, msg: Message)
        -> Result<(), _>
    {
        match ReadyClient::decode_command(msg){
            Ok(k)  => {self.perform_command(out, store, k);  Ok(())},
            Err(e) => {debug!("decode_command failed: {:?}", e); Err(e)}
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
    store: GraphStore,
    state: ClientState
}

type Client  = Framed<TcpStream, MessageCodec<OwnedMessage>>;

impl ServerHandler{
    fn on_message(mut self, handle: Handle, client: Client, msg: OwnedMessage) -> _ {
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
                            self.out.close(CloseCode::Unsupported);
                            Err("invalid graph ID"))
                        }
                    },
                    Err(e) => {
                        debug!("{:?}", e);
                        self.out.close(CloseCode::Protocol);
                        Err("expected graph choice, got something else"))
                    }
                }
        }
    }
    fn main_loop(mut self, handle: Handle, client: Client) { 
        handle.spawn(client.into_future().then({
            let handle = handle.clone();
            move |res|
            match res {
                Ok(m)  => self.on_message(handle, client, m),
                Err(e) => panic!()
            }
        }));
    }
}

#[derive(Default)]
pub struct ServerFactory{
    store: GraphStore
}

impl ServerFactory{
    pub fn new_connection(&self, handle: Handle, client: Client, headers: Headers) -> Result<()>{
        let addr = client.get_ref().peer_addr().unwrap();
        debug!("Connection from {} upgraded to WebSocket", addr);
        let graph_list = OwnedMessage::Binary(rmp_serde::encode::to_vec_named(&GraphList::from_graphstore(&self.store)).unwrap());
        let handler = ServerHandler{
            store: self.store.clone(),
            state: ClientState::AwaitingGraph
        };

        let main = client.send(graph_list).then({
            let handle = handle.clone();
            move |res|
                match res{
                    Ok(k)  => handler.main_loop(handle, client),
                    Err(e) => panic!()
                }
            });

        handle.spawn(main);

        Ok(())
    }
}
