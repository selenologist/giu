use serde::{Deserialize, Serialize};
//use rmp_serde;
use serde_json;
use ws::{listen, Handler, Factory, Sender, Handshake, Request, Response as WsResponse, Message, CloseCode};
use ws::{Error as WsError, ErrorKind as WsErrorKind, Result as WsResult};

use graph::{PossibleErr as GraphErr, *};

use std::thread;
use std::thread::{JoinHandle};
use std::fmt;
use std::result;

use rebuilder::{InvalidatedReceiver, InvalidatedReceiverMaker};

enum PossibleErr{
    Ws(WsError),
    String(String),
    GraphErr(GraphErr),
    JsonErr(serde_json::Error),
    Disp(Box<fmt::Display>),
    None
}

impl fmt::Display for PossibleErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::PossibleErr::*;
        match *self{
            Ws(ref w)       => w.fmt(f),
            String(ref s)   => s.fmt(f),
            GraphErr(ref g) => g.fmt(f),
            JsonErr(ref j)  => j.fmt(f),
            Disp(ref d)     => d.fmt(f),
            None            => (None).fmt(f)
        }
    }
}

type Result<T> = result::Result<T, PossibleErr>;

impl From<::std::option::NoneError> for PossibleErr{
    fn from(_: ::std::option::NoneError) -> PossibleErr{
        PossibleErr::None
    }
}

impl From<WsError> for PossibleErr{
    fn from(g: WsError) -> PossibleErr{
        PossibleErr::Ws(g)
    }
}

impl From<GraphErr> for PossibleErr{
    fn from(g: GraphErr) -> PossibleErr{
        PossibleErr::GraphErr(g)
    }
}

impl From<serde_json::Error> for PossibleErr{
    fn from(j: serde_json::Error) -> PossibleErr{
        PossibleErr::JsonErr(j)
    }
}


fn to_ws_err(e: PossibleErr, kind: WsErrorKind) -> WsError{
    use self::GraphErr::Ws as GWs;
    use self::PossibleErr::*;
    match e{
        Ws(w) | GraphErr(GWs(w)) => w,
        _ => WsError::new(kind,
                          format!("{}", e))
    }
}

fn to_ws(e: PossibleErr) -> WsError{
    to_ws_err(e, WsErrorKind::Internal)
}

impl Into<WsError> for PossibleErr{
    fn into(self) -> WsError{
       to_ws(self)
    }
}

fn decode_command(msg: Message) -> Result<Command>{
    match msg{
        Message::Text(t) => {
            Ok(serde_json::from_str(&t[..])?)
        },
        Message::Binary(..) =>
            Err(WsError::new(
                WsErrorKind::Protocol,
                format!("binary message received where expecting text JSON")).into())
    }
}

fn encode_response(response: Response) -> WsResult<Message>{
    match serde_json::to_string(&response){
        Ok(s)  => Ok(Message::Text(s)),
        Err(e) => Err(WsError::new(WsErrorKind::Internal, format!("encode_response failed {:?}", e)))
    }
}

fn encode_update<T: Into<Update>>(update: T) -> WsResult<Message>{
    match serde_json::to_string(&update.into()){
        Ok(s)  => Ok(Message::Text(s)),
        Err(e) => Err(WsError::new(WsErrorKind::Internal, format!("encode_update failed {:?}", e)))
    }
}

struct ClientCommon;
impl ClientCommon{
    fn on_open(out: &Sender, store: &GraphStore, id: GraphId,
               client_type: ClientType) -> Result<()>{
        if let Ok(_) = store.attach(id, client_type, out.clone()){
            trace!("Client supplied valid GraphId {}", id);
            Ok(())
        }
        else{
            let err = format!("GraphId {} does not exist", id);
            out.send(encode_response(Response::Err(DataValue::from(err.clone())))?)?;
            Ok(()) //Err(WsError::new(WsErrorKind::Protocol, err))
        }
    }
    fn on_command(_out: &Sender, store: &GraphStore,
                  command: &Command, graph: GraphId,
                  _client_type: ClientType) -> Result<Option<Response>> {
        use graph::Command::*; 
        let response = match *command{
            AddLink{ source: ref from, target: ref to } => {
                let graph = store.get(graph)?;
                
                let result = graph.add_link(from, to);
                match result{
                    Response::Ok => {
                        graph.repeat_to(ClientType::Both, encode_update(command.clone())?);
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
    fn on_open(out: &Sender, store: &GraphStore, id: GraphId) -> Result<Self>{
        ClientCommon::on_open(out, store, id, ClientType::Frontend)?;
        trace!("Frontend attached to GraphId {}", id);
        out.send(
            encode_update(
                Command::SetGraph{
                    graph: store.get(id)?.data.clone()
                }
            )?
        )?;
        Ok(FrontendClient{ graph: id })
    }

    fn on_command(&self, out: &Sender, store: &GraphStore,
                  command: &Command) -> Result<Response> {
        use graph::Command::*;
       
        if let Some(common) = ClientCommon::on_command(out, store, command, self.graph, ClientType::Frontend)?{
            return Ok(common);
        }
        match *command{
            _ => Err(WsError::new(
                        WsErrorKind::Protocol,
                        format!("Expected Frontend command, got {:?}",
                                command)).into())
        }
    }
}

#[derive(Copy,Clone)]
struct BackendClient{
    graph: GraphId
}

impl BackendClient{
    fn on_open(out: &Sender, store: &GraphStore, id: GraphId) -> Result<Self>{
        ClientCommon::on_open(out, store, id, ClientType::Backend)?;
        trace!("Backend attached to GraphId {}", id);
        Ok(BackendClient{ graph: id })
    } 

    fn on_command(&self, out: &Sender, store: &GraphStore,
                  command: &Command) -> Result<Response> {
        use graph::Command::*;
        let client_type = ClientType::Backend;
       
        if let Some(common) = ClientCommon::on_command(out, store, command, self.graph, client_type.clone())?{
            return Ok(common);
        }
        
        match *command{
            SetGraph{ ref graph } => Ok({
                trace!("set graph {:?}", graph);
                store.set_graph(self.graph, graph.clone())?;
                store.repeat_to(self.graph, client_type.opposite(),
                                encode_update(command.clone())?)?;
                Response::Ok
            }),
            _ => Err(WsError::new(WsErrorKind::Protocol,
                                  format!("Expected Backend command, got {:?}",
                                          command)).into())
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

impl ServerHandler{
    fn on_open_inner(&mut self, hs: Handshake) -> Result<()>{
        if let Some(ip_addr) = hs.peer_addr {
            let ip_string = format!("{}", ip_addr);
            info!("{:>20} - connection {:?} established", ip_string, self.out.token());
            self.addr = ip_string;
        }
        else{
            debug!("Connection without IP address?");
        }

        self.out.send(
            serde_json::to_string(
                &self.store.list())?
            )?;

        Ok(())
    }
    fn on_message_inner(&mut self, msg: Message) -> Result<()> {
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
                    BackendAttach { id } => {
                        let id = match id{
                            Some(id) if self.store.contains_key(id) => id,
                            Some(id) => self.store.empty_at(id),
                            None     => self.store.new_empty()
                        };
                        Backend(BackendClient::on_open(out, store, id)?)
                    },
                    _ => 
                      return Err(WsError::new(WsErrorKind::Protocol,
                                         "Expected FrontendAttach or BackendAttach, got something else").into())
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
}

impl Handler for ServerHandler{
    fn on_open(&mut self, hs: Handshake) -> WsResult<()>{
        self.on_open_inner(hs).map_err(|e|e.into())
    }

    fn on_request(&mut self, req: &Request) -> WsResult<WsResponse> {
        let mut res = WsResponse::from_request(req)?;

        let protocol_name = "selenologist-node-editor";
        res.set_protocol(protocol_name);

        Ok(res)
    }

    fn on_message(&mut self, msg: Message) -> WsResult<()> {
        self.on_message_inner(msg).map_err(|e|e.into())
    }
    
    fn on_close(&mut self, code: CloseCode, reason: &str){
        use self::ClientState::*;
        trace!("Closing connection {:?} because {:?} {}", self.addr, code, reason);
        self.out.close(code);
        match self.state{
            Backend(BackendClient{ graph }) |
            Frontend(FrontendClient{ graph }) => {
                self.store.remove_listener(graph, self.out.token().0);
            }
            _ => {}
        }
    }
}

struct ServerFactory{
    store: GraphStore,
    invalidated_rx_maker: InvalidatedReceiverMaker
}

impl Factory for ServerFactory{
    type Handler = ServerHandler;

    fn connection_made(&mut self, out: Sender) -> Self::Handler{
        let mut invalidated_rx = self.invalidated_rx_maker.add_rx();
        let inform_out = out.clone();
        // ahahah look at this, it leaks threads because ws::Sender has no
        // disconenction mechanism. I just really wanted this to work.
        thread::spawn(move || loop {
            match invalidated_rx.recv(){
                Ok(s) => {
                    if s.ends_with("main.js"){
                        inform_out.send(
                            encode_update(Command::Reload).unwrap()
                        ).unwrap();
                        // this client will reload now so we may as well kill this thread.
                        // We'll still leak any threads that don't get this far though.
                        return
                    }
                },
                Err(e) => panic!(e)
            }});
        ServerHandler{
            out,
            store: self.store.clone(),
            state: ClientState::AwaitingType,
            addr:  "0.0.0.0:0".into()
        }
    }
}

pub fn launch_thread(invalidated_rx_maker: InvalidatedReceiverMaker)
    -> JoinHandle<()>
{
    use std::collections::BTreeMap;
    let d = GraphData{
        nodes: {
            let mut map = BTreeMap::new();
            map.insert("TestLabel".into(), Node::Label{data: "TestData".into()});
            map.insert("InPortNode".into(), Node::InPort);
            map.insert("OutPortNode".into(), Node::OutPort);
            map
        },
        links: BTreeMap::new(),
        data: {
            let mut map = BTreeMap::new();
            map.insert("TestData".into(), DataValue::from(String::from("Test Node")));
            map
        }
    };

    let s = serde_json::to_string(&d).unwrap();
    println!("looks like {}", s);

    thread::Builder::new()
        .name("websocket".into())
        .spawn(move || {
        let mut factory = ServerFactory{
            store: GraphStore::default(),
            invalidated_rx_maker
        };
        let listen_addr = "127.0.0.1:3001";
        info!("Attempting to listen on {}", listen_addr);
        listen(listen_addr, |out| factory.connection_made(out)).unwrap()
    }).unwrap()
}
