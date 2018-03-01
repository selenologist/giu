use serde::{Deserialize, Serialize};
use rmp_serde;
use rmp_serde::{Deserializer, Serializer};
use ws::{listen, Handler, Factory, Sender, Handshake, Message, CloseCode};

use graph::*;

use std::cell::RefCell;
use std::thread;
use std::thread::{Thread, JoinHandle};
use std::io;

#[derive(Copy,Clone)]
struct ReadyClient{
    graph: GraphId
}

impl ReadyClient{
    fn decode_command(msg: Message) -> Result<Command, ::ws::Error>{
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
                       command: Command) -> Result<(), ::ws::Error> {
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
        -> Result<(), ::ws::Error>
    {
        match ReadyClient::decode_command(msg){
            Ok(k)  => {self.perform_command(out, store, k);  Ok(())},
            Err(e) => {println!("decode_command failed: {:?}", e); Err(e)}
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
    state: ClientState
}

impl Handler for ServerHandler{
    fn on_message(&mut self, msg: Message) -> ::ws::Result<()> {
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
                            Err(::ws::Error::new(::ws::ErrorKind::Protocol, "invalid graph ID"))
                        }
                    },
                    Err(e) => {
                        println!("{:?}", e);
                        self.out.close(CloseCode::Protocol);
                        Err(::ws::Error::new(::ws::ErrorKind::Protocol, "expected graph choice, got something else"))
                    }
                }
        }
    }
}

#[derive(Default)]
struct ServerFactory{
    store: GraphStore
}

impl Factory for ServerFactory{
    type Handler = ServerHandler;

    fn connection_made(&mut self, out: Sender) -> Self::Handler{
        println!("[websocket] Connection received");
        out.send(rmp_serde::encode::to_vec_named(&GraphList::from_graphstore(&self.store)).unwrap());
        ServerHandler{
            out,
            store: self.store.clone(),
            state: ClientState::AwaitingGraph
        }
    }
}

pub fn launch_thread() -> JoinHandle<()>{
    thread::spawn(move ||{
        let mut factory = ServerFactory::default();
        listen("127.0.0.1:3001", |out| factory.connection_made(out)).unwrap()
    })
}
