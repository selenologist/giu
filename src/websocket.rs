use serde::{Deserialize, Serialize};
use rmp_serde;
use rmp_serde::{Deserializer, Serializer};
use ws::{listen, Handler, Factory, Sender, Handshake, Request, Response as WsResponse, Message, CloseCode};
use ws::{Error as WsError, ErrorKind as WsErrorKind, Result as WsResult};
use futures::future::{Future, IntoFuture};

use graph::*;

use std::thread;
use std::thread::{JoinHandle};
use std::io::Cursor;
use std::fmt::Display;

fn to_ws_err<D: Display>(thing: D, kind: WsErrorKind) -> WsError{
    WsError::new(kind,
                 format!("{}", thing))
}

fn to_ws_internal<D: Display>(thing: D) -> WsError{
    to_ws_err(thing, WsErrorKind::Internal)
}

fn decode_command(msg: Message) -> WsResult<Command>{
    use rmpv;
    use std::io::Read;
    match msg{
        Message::Text(..) =>
            Err(WsError::new(
                WsErrorKind::Protocol,
                String::from("text received where binary msgpack expected"))),
        Message::Binary(b) => {
            let res: Result<Command, _> = rmp_serde::decode::from_slice(&b[..]);
            match res{
                Ok(k)  => Ok(k),
                Err(e) => Err(WsError::new(WsErrorKind::Protocol,
                                           format!("decode_command failed {:?}, val: {:?}",
                                                   e,{
                    let mut cur = Cursor::new(&b[..]);
                    let v: rmpv::Value = rmpv::decode::value::read_value(&mut cur).unwrap();
                    v
                })))
            }
       }
   }
}

fn encode_response(response: Response) -> WsResult<Message>{
    match rmp_serde::encode::to_vec_named(&response){
        Ok(v)  => Ok(Message::Binary(v)),
        Err(e) => Err(WsError::new(WsErrorKind::Internal, format!("encode_response failed {:?}", e)))
    }
}

fn encode_update<T: Into<Update>>(update: T) -> WsResult<Message>{
    match rmp_serde::encode::to_vec_named(&update.into()){
        Ok(v)  => Ok(Message::Binary(v)),
        Err(e) => Err(WsError::new(WsErrorKind::Internal, format!("encode_update failed {:?}", e)))
    }
}

struct ClientCommon;
impl ClientCommon{
    fn on_open(out: &Sender, store: &GraphStore, id: GraphId,
               client_type: ClientType) -> WsResult<()>{
        if store.contains_key(id){
            trace!("Client supplied valid GraphId {}", id);
            if let Err(e) = store.attach(id, client_type, out.clone()){
                return Err(WsError::new(WsErrorKind::Internal, format!("{}", e)));
            }
            if client_type == ClientType::Frontend {
                let graph =
                    store
                    .get(id)
                    .unwrap();
                out.send(
                    encode_update(
                        Command::SetGraph{
                            graph: graph.data.clone()
                        }
                    )?
                )?;
            }
            Ok(())
        }
        else{
            let err = format!("GraphId {} does not exist", id);
            out.send(encode_response(Response::Err(DataValue::from(err.clone())))?)?;
            Ok(()) //Err(WsError::new(WsErrorKind::Protocol, err))
        }
    }
    fn on_command(out: &Sender, store: &GraphStore,
                  command: &Command, graph: GraphId,
                  client_type: ClientType) -> WsResult<Option<Response>> {
        use graph::Command::*; 
        let response = match *command{
            AddLink{ from: (ref from_node, ref from_port), to: (ref to_node, ref to_port) } => {
                let graph = store.get(graph).unwrap();
                
                let result = graph.add_link(&from_node, &from_port, &to_node, &to_port);
                match result{
                    Response::Ok => {
                        graph.repeat_to(client_type.opposite(), encode_update(command.clone())?);
                        Response::Ok
                    }
                    _ => result
                }
            },
            _ => {return Ok(None)}
        };
        Ok(Some(response))
    }
}

#[derive(Copy,Clone)]
struct FrontendClient{
    graph: GraphId
}

impl FrontendClient{
    fn on_open(out: &Sender, store: &GraphStore, id: GraphId) -> WsResult<Self>{
        ClientCommon::on_open(out, store, id, ClientType::Frontend)?;
        trace!("Frontend attached to GraphId {}", id);
        Ok(FrontendClient{ graph: id })
    }

    fn on_command(&self, out: &Sender, store: &GraphStore,
                  command: &Command) -> WsResult<Response> {
        use graph::Command::*;
       
        if let Some(common) = ClientCommon::on_command(out, store, command, self.graph, ClientType::Frontend)?{
            return Ok(common);
        }
        match *command{
            _ => {
                return Err(
                      WsError::new(WsErrorKind::Protocol,
                                   format!("Expected Frontend command, got {:?}",
                                           command)));
            }
        }
    }
}

#[derive(Copy,Clone)]
struct BackendClient{
    graph: GraphId
}

impl BackendClient{
    fn on_open(out: &Sender, store: &GraphStore, id: GraphId) -> WsResult<Self>{
        ClientCommon::on_open(out, store, id, ClientType::Backend)?;
        trace!("Backend attached to GraphId {}", id);
        Ok(BackendClient{ graph: id })
    } 

    fn on_command(&self, out: &Sender, store: &GraphStore,
                  command: &Command) -> WsResult<Response> {
        use graph::Command::*;
        let client_type = ClientType::Backend;
       
        if let Some(common) = ClientCommon::on_command(out, store, command, self.graph, client_type.clone())?{
            return Ok(common);
        }
        
        match *command{
            SetGraph{ ref graph } => Ok({
                trace!("set graph {:?}", graph);
                match store.set_graph(self.graph, graph.clone()){
                    Ok(k) => {},
                    Err(e) => return Err(to_ws_internal(e))
                };
                match store.get(self.graph){
                    Ok(g)  => g.repeat_to(client_type.opposite(), encode_update(command.clone())?),
                    Err(e) => return Err(to_ws_internal(e))
                };
                Response::Ok}),
            _ => {
                return Err(
                      WsError::new(WsErrorKind::Protocol,
                                   format!("Expected Backend command, got {:?}",
                                           command)));
            }
        }
    }
}

#[derive(Copy,Clone)]
enum ClientState{
    Frontend(FrontendClient),
    Backend(BackendClient),
    AwaitingType
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
            info!("{:>20} - connection {:?} established", ip_string, self.out.token());
            self.addr = ip_string;
        }
        else{
            debug!("Connection without IP address?");
        }

        self.out.send(rmp_serde::encode::to_vec_named(
            &self.store.list())
            .unwrap())?;

        Ok(())
    }

    fn on_request(&mut self, req: &Request) -> WsResult<WsResponse> {
        let mut res = WsResponse::from_request(req)?;

        let protocol_name = "selenologist-node-editor";
        res.set_protocol(protocol_name);

        Ok(res)
    }

    fn on_message(&mut self, msg: Message) -> WsResult<()> {
        use self::ClientState::*;
        use graph::Command::{FrontendAttach, BackendAttach};
        let command = decode_command(msg)?;
        let response = match self.state.clone() {
            Frontend(client) =>
                client.on_command(&self.out, &self.store, &command)?,
            Backend(client) =>
                client.on_command(&self.out, &self.store, &command)?,
            AwaitingType => {
                let out   = &self.out;
                let store = &self.store;
                let state = match command{
                    FrontendAttach{ id } =>
                        Frontend(FrontendClient::on_open(out, store, id)?),
                    BackendAttach { id } =>
                        if let Some(id) = id{
                            if !self.store.contains_key(id){
                                self.store.empty_at(id).unwrap();
                            }
                            Backend(BackendClient::on_open(out, store, id)?)
                        }
                        else{
                            Backend(BackendClient::on_open(out, store,
                                self.store.new_empty().unwrap()
                            )?)
                        },
                    _ => {
                        return Err(
                            WsError::new(WsErrorKind::Protocol,
                                         "Expected FrontendAttach or BackendAttach, got something else"));
                    }
                };
                self.state = state;
                Response::Ok
            }
        };
        if response != Response::Ok{ // don't generate Ok messages, they're pointless and hard to coordinate
            self.out.send(encode_response(response)?)?
        }
        Ok(())
    }
    
    fn on_close(&mut self, code: CloseCode, reason: &str){
        use self::ClientState::*;
        trace!("Closing connection {:?} because {:?} {}", self.addr, code, reason);
        self.out.close(code);
        match self.state{
            Backend(BackendClient{ graph }) |
            Frontend(FrontendClient{ graph }) => {
                self.store.remove_listener(graph, self.out.token().0)
            }
            _ => {}
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
            state: ClientState::AwaitingType,
            addr:  "0.0.0.0:0".into()
        }
    }
}

pub fn launch_thread() -> JoinHandle<()>{
    use std::collections::BTreeMap;
    use rmpv;
    let d = GraphData{
        nodes: {
            let mut map = BTreeMap::new();
            map.insert("TestLabel".into(), Node::Label{id: "TestData".into()});
            map.insert("InPortNode".into(), Node::InPort{id: "TestInPort".into()});
            map.insert("OutPortNode".into(), Node::OutPort{id: "TestOutPort".into()});
            map
        },
        links: BTreeMap::new(),
        data: {
            let mut map = BTreeMap::new();
            map.insert("TestData".into(), DataValue::from(String::from("Test Node")));
            map
        }
    };

    let v = rmp_serde::encode::to_vec_named(&d).unwrap();
    let mut c = Cursor::new(v);
    let r = rmpv::decode::read_value(&mut c);
    println!("looks like {:?}", r);


    thread::Builder::new()
        .name("websocket".into())
        .spawn(move || {
        let mut factory = ServerFactory::default();
        let listen_addr = "127.0.0.1:3001";
        info!("Attempting to listen on {}", listen_addr);
        listen(listen_addr, |out| factory.connection_made(out)).unwrap()
    }).unwrap()
}
