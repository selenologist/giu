use serde::{Deserialize, Serialize, Serializer as SerdeSerializer, Deserializer as SerdeDeserializer};
//use rmp_serde::{Deserializer, Serializer};
//use rmpv;
use serde_json::{Deserializer, Serializer};

use std::collections::{BTreeMap, BTreeSet};
use std::cell::{Cell, RefCell,RefMut};
use std::rc::Rc;

use ws::{Message as WsMessage, Result as WsResult, Error as WsError, ErrorKind as WsErrorKind, Sender as WsSender};

pub type GraphId    = u32;
pub type NodeId     = String;
pub type PortId     = String;
pub type DataId     = String;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag="type")]
pub enum Node{
    Label     { data: DataId },
    Labelled  { data: DataId, nodes: BTreeMap<NodeId, Node> },
    Container { nodes: Vec<Node> },
    Knob      { data: DataId },
    Button    { data: DataId },
    BiPort    { port: PortId },
    InPort    { port: PortId },
    OutPort   { port: PortId }
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

    pub fn add_link(&self, from_port: &PortId, to_port: &PortId)
        -> Response
    {
        let mut data = self.data.borrow_mut();

        let mut entry = data.links.entry(from_port.clone())
            .or_insert(BTreeSet::new());
        let is_new = entry.insert(to_port.clone());
        if !is_new {
            Response::Warn(DataValue::from(format!(
                "Link {} -> {} already exists",
                from_port, to_port)))
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
   pub fn get(&self, id: GraphId) -> Result<Graph, String>{
        match self.0.borrow().get(&id){
            Some(s) => Ok(s.clone()),
            None    => Err(format!("No such graph {:?}", id))
        }
    }
    pub fn contains_key(&self, id: GraphId) -> bool{
        self.0.borrow().contains_key(&id)
    }
    pub fn insert(&self, id: GraphId, graph: Graph) -> Result<(), String>{
        self.0.borrow_mut().insert(id, graph);
        Ok(())
    }
    pub fn remove_listener(&self, graph: GraphId, token: usize){
        self.get(graph).unwrap().remove_listener(token);
    }
   
    pub fn new(&self, graph: Graph) -> Result<GraphId, String>{
        let id = self.new_id();
        self.insert(id, graph)?;
        Ok(id)
    }
    pub fn new_empty(&self) -> Result<GraphId, String>{
        self.new(Graph::default())
    }
    pub fn empty_at(&self, id: GraphId) -> Result<GraphId, String>{
        self.insert(id, Graph::default())?;
        Ok(id)
    }

    pub fn attach(&self, id: GraphId, client_type: ClientType, sender: WsSender) -> Result<(), String>{
        let g = self.get(id)?;
        g.listeners.borrow_mut().insert(sender.token().0, ((client_type, sender)));
        Ok(())
    }
    pub fn set_graph(&self, id: GraphId, graph_data: Rc<RefCell<GraphData>>) -> Result<(), String>{
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
#[serde(tag="type")]
pub enum Command{
    AddLink {from: PortId, to: PortId},
    DelLink {from: PortId, to: PortId},
    SetData {id:   DataId,  value: DataValue},
    SetGraph{graph: Rc<RefCell<GraphData>>},
    FrontendAttach {id: GraphId},
    BackendAttach  {id: Option<GraphId>}
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag="type")]
pub enum Update{
    Command {val: Command},
}

impl From<Command> for Update{
    fn from(val: Command) -> Self{
        Update::Command{ val }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DataValue{
    Nil{val: Option<()>},
    Int{val: i32},
    Float{val: f32},
    String{val: String},
    Graph{val: Rc<RefCell<GraphData>>}
}

impl From<()> for DataValue{
    fn from(_val: ()) -> Self{
        DataValue::Nil{ val: None }
    }
}

impl From<i32> for DataValue{
    fn from(val: i32) -> DataValue{
        DataValue::Int{ val }
    }
}

impl From<String> for DataValue{
    fn from(val: String) -> DataValue{
        DataValue::String{ val }
    }
}

impl From<Rc<RefCell<Graph>>> for DataValue{
    fn from(val: Rc<RefCell<Graph>>) -> Self{
        let v = val.clone();
        let v = v.borrow();
        DataValue::Graph{ val: v.data.clone() }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag="type")]
pub enum Response{
    Ok,
    Warn(DataValue),
    Err(DataValue)
}

