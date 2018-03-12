use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;

use rebuilder::InvalidatedReceiver;

use file::InMemoryFile;

#[derive(Clone)]
pub struct FileCache(Rc<RefCell<(HashMap<String, Rc<InMemoryFile>>, InvalidatedReceiver)>>);

impl FileCache{
    pub fn new(invalidated_rx: InvalidatedReceiver) -> FileCache{
        FileCache(Rc::new(RefCell::new((HashMap::new(), invalidated_rx))))
    }

    pub fn try_invalidate(&self){
        use std::sync::mpsc::TryRecvError::Empty;
        let mut b = self.0.borrow_mut();
        loop{
            match b.1.try_recv(){
                Ok(s)  => {
                    trace!("Removing invalidated file {} from cache", s);
                    b.0.remove(&s); // we don't care if the key actually existed, so long as it's gone
                },
                Err(e) => {
                    if e == Empty{
                        break; // no more messages, just exit
                    }
                    else{
                        panic!("Cache invalidation message receiver disconnected!");
                    }
                }
            }
        }
    }

    pub fn get(&self, key: &String) -> Option<Rc<InMemoryFile>>{
        self.try_invalidate();

        if let Some(v) = self.0.borrow().0.get(key){
            Some(v.clone())
        }
        else{
            None
        }
    }

    pub fn insert(&self, key: String, value: Rc<InMemoryFile>){
        trace!("Storing {} in cache", key);
        self.0.borrow_mut().0.insert(key, value);
    }
}


