use serde::{Deserialize, Serialize};
use rmp_serde::{Deserializer, Serializer};
use rmpv;

use std::collections::BTreeMap;
use std::cell::{Cell, RefCell,RefMut};
use std::rc::Rc;

pub type GraphId    = u32;
pub type NodeId     = u32;
pub type PortId     = u32;
pub type FullPortId = (NodeId, PortId);
pub type DataId     = u32;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub enum Node{
    Label(DataId),
    Labelled(DataId, Box<Node>),
    Container(Vec<Node>),
    Knob(DataId),
    Button(DataId),
    BiPort(PortId),
    InPort(PortId),
    OutPort(PortId)
}

impl Node{
    pub fn find_port(&self, port: PortId) -> Option<()>{
        use self::Node::*;
        match *self{
            BiPort(p)  |
            InPort(p)  |
            OutPort(p) => {
                if p == port {
                    Some(())
                }
                else{
                    None
                }
            },
            Labelled(.., ref n) =>
                n.find_port(port),
            Container(ref v) => {
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


#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Graph{
    pub nodes: BTreeMap<NodeId, Node>,
    pub links: BTreeMap<FullPortId, FullPortId>,
    pub data:  BTreeMap<DataId, DataValue>
}

impl Graph{
    pub fn add_link(&mut self,
                from_node: NodeId, from_port: PortId,
                to_node:   NodeId, to_port:   PortId)
        -> Response
    {
        let all_ports_valid: bool = {
            let from = self.nodes.get(&from_node);
            let to   = self.nodes.get(&to_node);

            from.is_some() &&
              to.is_some() &&
            from.unwrap().find_port(from_port).is_some() &&
              to.unwrap().find_port(to_port  ).is_some()
        };

        if all_ports_valid{
            let last_node = self.links.insert(
                (from_node, from_port),
                (to_node,   to_port)
            );
            if last_node.is_some() &&
               last_node.unwrap() == (to_node, to_port) {
                Response::Warn(format!(
                    "Link ({},{}) -> ({},{}) already exists",
                    from_node, from_port, to_node, to_port))
            }
            else{
                Response::Ok
            }
        }
        else{
            Response::Err(format!(
                "Link ({},{}) -> ({},{}) refers to invalid nodes or ports",
                from_node, from_port, to_node, to_port))
        }
    }
}

#[derive(Clone, Default)]
pub struct GraphStore(pub Rc<RefCell<BTreeMap<GraphId, Rc<RefCell<Graph>>>>>,
                      Rc<Cell<GraphId>>);

impl GraphStore{
    pub fn get(&self, id: GraphId) -> Result<Rc<RefCell<Graph>>, String>{
        match self.0.borrow().get(&id){
            Some(s)  => Ok(s.clone()),
            None => Err(format!("No such graph {:?}", id))
        }
    }
    pub fn contains_key(&self, id: GraphId) -> bool{
        self.0.borrow().contains_key(&id)
    }
    fn new_id(&self) -> GraphId{
        let new_id = self.1.get();
        self.1.set(new_id + 1);
        new_id
    }
    pub fn new(&self, graph: Rc<RefCell<Graph>>) -> Result<GraphId, ()>{
        let id = self.new_id();
        self.0.borrow_mut().insert(id, graph);
        Ok(id)
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct GraphList{
    graphs: Vec<GraphId>
}

impl GraphList{
    pub fn from_graphstore(gs: &GraphStore) -> GraphList{
        let v = gs.0.borrow().keys().cloned().collect();
        GraphList{
            graphs: v
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub enum Command{
    AddLink(FullPortId, FullPortId),
    DelLink(FullPortId, FullPortId),
    SetData(DataId, DataValue),
    SetGraph(Box<Graph>)
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub enum DataValue{
    Int(i32),
    Float(f32),
    String(String)
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub enum Response{
    Ok,
    Warn(String),
    Err(String)
}

