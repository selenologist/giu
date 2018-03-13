use serde::{Deserialize, Serialize, Serializer as SerdeSerializer, Deserializer as SerdeDeserializer};
use rmp_serde::{Deserializer, Serializer};
use rmpv;

use std::collections::BTreeMap;
use std::cell::{Cell, RefCell,RefMut};
use std::rc::Rc;

use ws::{Message as WsMessage, Result as WsResult, Error as WsError, ErrorKind as WsErrorKind, Sender as WsSender};

pub type GraphId    = u32;
pub type NodeId     = String;
pub type PortId     = String;
pub type FullPortId = (NodeId, PortId);
pub type DataId     = String;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag="type")]
pub enum Node{
    Label{ id: DataId },
    Labelled{ id: DataId, nodes: Box<Node> },
    Container{ nodes: Vec<Node> },
    Knob{ id: DataId },
    Button{ id: DataId },
    BiPort{ id: PortId },
    InPort{ id: PortId },
    OutPort{ id: PortId }
}

impl Node{
    pub fn find_port(&self, port: &PortId) -> Option<()>{
        use self::Node::*;
        match *self{
            BiPort{ref id}  |
            InPort{ref id}  |
            OutPort{ref id} => {
                if id == port {
                    Some(())
                }
                else{
                    None
                }
            },
            Labelled{ nodes: ref n, .. } =>
                n.find_port(port),
            Container{ nodes: ref v } => {
                if v.iter().any(|ref n| n.find_port(port).is_some()){
                    Some(())
                }
                else{
                    None
                }
            },
            _ => None
        }
    }
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
    pub links: BTreeMap<FullPortId, FullPortId>,
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

    pub fn add_link(&self,
                from_node: &NodeId, from_port: &PortId,
                to_node:   &NodeId, to_port:   &PortId)
        -> Response
    {
        let mut data = self.data.borrow_mut();
        let all_ports_valid: bool = {
            let from = data.nodes.get(from_node);
            let to   = data.nodes.get(to_node);

            from.is_some() &&
              to.is_some() &&
            from.unwrap().find_port(from_port).is_some() &&
              to.unwrap().find_port(to_port  ).is_some()
        };

        if all_ports_valid{
            let last_node = data.links.insert(
                (from_node.clone(), from_port.clone()),
                (to_node.clone(),   to_port.clone())
            );
            match last_node{
                Some((ref last_to_node, ref last_to_port))
                => // XXX blah we should handle multiple node connections if (last_to_node, last_to_port) == (to_node, to_port) =>
                    Response::Warn(DataValue::from(format!(
                        "Link ({},{}) -> ({},{}) already exists",
                        from_node, from_port, to_node, to_port))),
                None => Response::Ok
            }
        }
        else{
            Response::Err(DataValue::from(format!(
                "Link ({},{}) -> ({},{}) refers to invalid nodes or ports",
                from_node, from_port, to_node, to_port)))
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
    AddLink {from: FullPortId, to: FullPortId},
    DelLink {from: FullPortId, to: FullPortId},
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

