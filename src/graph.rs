use std::collections::{BTreeMap, BTreeSet};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::fmt;

use ws::{Message as WsMessage, Error as WsError, Sender as WsSender};

pub type GraphId    = u32;
pub type NodeId     = String;
pub type PortId     = String;
pub type DataId     = String;

#[derive(Debug)]
pub enum PossibleErr{
    Ws(WsError),
    String(String),
    None
}

impl fmt::Display for PossibleErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::PossibleErr::*;
        match *self{
            Ws(ref w)     => w.fmt(f),
            String(ref s) => s.fmt(f),
            None          => (None).fmt(f)
        }
    }
}
impl Into<PossibleErr> for String{
    fn into(self) -> PossibleErr{
        PossibleErr::String(self)
    }
}
impl Into<PossibleErr> for ::std::option::NoneError{
    fn into(self) -> PossibleErr{
        PossibleErr::None
    }
}

impl From<WsError> for PossibleErr{
    fn from(g: WsError) -> PossibleErr{
        PossibleErr::Ws(g)
    }
}

type Result<T> = ::std::result::Result<T, PossibleErr>;


#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag="type")]
pub enum Node{
    Label     { data: DataId },
    Labelled  { data: DataId, nodes: BTreeMap<NodeId, Node> },
    Container { nodes: Vec<Node> },
    Knob      { data: DataId },
    Button    { data: DataId },
    BiPort,
    InPort,
    OutPort
}

#[derive(Copy, Clone, Debug)]
pub enum ClientType{
    Frontend,
    Backend,
    Both
}

impl PartialEq for ClientType{
    fn eq(&self, other: &Self) -> bool{
        use self::ClientType::*;
        match *self{
            Both => true,
            Frontend =>
                match *other{
                    Frontend => true,
                    Backend  => false,
                    Both     => true,
                },
            Backend =>
                match *other{
                    Frontend => false,
                    Backend  => true,
                    Both     => true,
                }
        }
    }
    fn ne(&self, other: &Self) -> bool{
        !self.eq(other)
    }
}

impl ClientType{
    pub fn opposite(&self) -> ClientType{
        use self::ClientType::*;
        match *self{
            Frontend => Backend,
            Backend  => Frontend,
            Both     => Both
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Deserialize, Serialize)]
pub struct GraphData{
    pub nodes: BTreeMap<NodeId, Node>,
    pub links: BTreeMap<PortId, BTreeSet<PortId>>,
    pub data:  BTreeMap<DataId, DataValue>
}

#[derive(Clone, Default)]
pub struct Graph{
    pub data:      Rc<RefCell<GraphData>>,
    pub listeners: Rc<RefCell<BTreeMap<usize, (ClientType, WsSender)>>>
}

impl Graph{
    pub fn repeat_to(&self, client_type: ClientType, msg: WsMessage){
        trace!("there are {} listeners", self.listeners.borrow().len());
        self.listeners
            .borrow()
            .values()
            .for_each(|&(ref c, ref s): &(ClientType, WsSender)|{
                if c == &client_type {
                    trace!("sent msg to {:?}", client_type);
                    match s.send(msg.clone()){
                        Ok(_)  => {},
                        Err(e) => {debug!("failed to repeat message to listener {:?}", e);}
                    }
                }
            });
    }
    pub fn remove_listener(&self, token: usize){
        self.listeners.borrow_mut().remove(&token);
    }

    pub fn add_link(&self, source_port: &PortId, target_port: &PortId)
        -> Response
    {
        let mut data = self.data.borrow_mut();

        let entry = data.links.entry(source_port.clone())
            .or_insert(BTreeSet::new());
        let is_new = entry.insert(target_port.clone());
        if !is_new {
            Response::Warn(DataValue::from(format!(
                "Link {} -> {} already exists",
                source_port, target_port)))
        }
        else{
            Response::Ok
        }
    }
}

#[derive(Clone, Default)]
pub struct GraphStore(pub Rc<RefCell<BTreeMap<GraphId, Graph>>>,
                      Rc<Cell<GraphId>>);

impl GraphStore{
    fn new_id(&self) -> GraphId{
        let new_id = self.1.get();
        self.1.set(new_id + 1);
        new_id
    }
    pub fn get(&self, id: GraphId) -> Result<Graph>{
        if let Some(g) = self.0.borrow().get(&id){
            Ok(g.clone())
        }
        else{
            Err(PossibleErr::None)
        }
    }
    pub fn contains_key(&self, id: GraphId) -> bool{
        self.0.borrow().contains_key(&id)
    }
    pub fn insert(&self, id: GraphId, graph: Graph){
        self.0.borrow_mut().insert(id, graph);
    }
    pub fn remove_listener(&self, graph: GraphId, token: usize) -> Result<()>{
        Ok(self.get(graph)?.remove_listener(token))
    }
   
    pub fn new(&self, graph: Graph) -> GraphId{
        let id = self.new_id();
        self.insert(id, graph);
        id
    }
    pub fn new_empty(&self) -> GraphId{
        self.new(Graph::default())
    }
    pub fn empty_at(&self, id: GraphId) -> GraphId{
        self.insert(id, Graph::default());
        id
    }

    pub fn attach(&self, id: GraphId, client_type: ClientType, sender: WsSender) -> Result<usize>{
        let g = self.get(id)?;
        let mut l = g.listeners.borrow_mut();
        l.insert(sender.token().0, (client_type, sender));
        Ok(l.len())
    }
    pub fn repeat_to(&self, id: GraphId, client_type: ClientType, msg: WsMessage) -> Result<()>{
        self.get(id)?
            .repeat_to(client_type, msg);
        Ok(())
    }
    pub fn set_graph(&self, id: GraphId, graph_data: Rc<RefCell<GraphData>>) -> Result<()>{
        let g = self.get(id)?;
        g.data.swap(&graph_data);
        Ok(())
    }

    pub fn list(&self) -> GraphList{
        let v = self.0.borrow().keys().cloned().collect();
        GraphList{
            list: v
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct GraphList{
    list: Vec<GraphId>
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "_")]
pub enum Command{
    AddLink {source: PortId, target: PortId},
    DelLink {source: PortId, target: PortId},
    SetData {id:     DataId, value:  DataValue},
    SetGraph{graph: Rc<RefCell<GraphData>>},
    FrontendAttach {id: GraphId},
    BackendAttach  {id: Option<GraphId>},
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
// externally tagged (serde default)
pub enum Update{
    Command(Command),
    Response(Response)
}

impl From<Command> for Update{
    fn from(val: Command) -> Self{
        Update::Command(val)
    }
}

impl From<Response> for Update{
    fn from(val: Response) -> Self{
        Update::Response(val)
    }
}


#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DataValue{
    Nil,
    Int(i32),
    Float(f32),
    String(String),
    Graph(Rc<RefCell<GraphData>>)
}

impl From<()> for DataValue{
    fn from(_val: ()) -> Self{
        DataValue::Nil
    }
}

impl From<i32> for DataValue{
    fn from(val: i32) -> DataValue{
        DataValue::Int(val)
    }
}

// hack around cycle caused by () impl'ing Into<String>
pub trait ShouldDataValueFromString{}
impl     ShouldDataValueFromString for String{}
impl<'a> ShouldDataValueFromString for &'a str{}
impl<S> From<S> for DataValue where S: Into<String> + ShouldDataValueFromString{
    fn from(val: S) -> DataValue{
        DataValue::String(val.into())
    }
}

impl From<Rc<RefCell<Graph>>> for DataValue{
    fn from(val: Rc<RefCell<Graph>>) -> Self{
        let v = val.clone();
        let v = v.borrow();
        DataValue::Graph(v.data.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Response{
    Ok,
    Warn(DataValue),
    Err(DataValue)
}
