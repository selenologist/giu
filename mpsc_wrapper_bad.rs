// This doesn't work because there's no way to signal when the other end has sent a message
//
struct SyncSenderWrapper<T: Send>{
    sender: SyncSender<T>,
    thing:  T
}

impl<T: Send> SyncSenderWrapper<T>{
    pub fn send(sender: SyncSender<T>, thing: T) -> SyncSenderWrapper<T>{
        SyncSenderWrapper{
            sender,
            thing
        }
    }
}

// Temporarily make this Clone so that we can actually send from a &mut
impl<T: Send + Clone> Future for SyncSenderWrapper<T>{
    type Item  = ();
    type Error = TrySendError<T>; // only returned on Disconnected, Full is just a failed poll that will be retried
    fn poll(&mut self) -> Result<Async<Self::Item>, Self::Error>{
        match self.sender.try_send(self.thing.clone()) {
            Ok(k)  => Ok(Async::Ready(k)),
            Err(e) =>
                if let TrySendError::Full(_t) = e{
                    //self.thing = t;
                    Ok(Async::NotReady)
                }
                else{
                    Err(e) // this is a TrySendError::Disconnected
                }
        }
    }
}

struct ReceiverWrapper<T: Send>{
    receiver: Rc<Receiver<T>>
}

impl<T: Send> ReceiverWrapper<T>{
    pub fn recv(receiver: Rc<Receiver<T>>) -> ReceiverWrapper<T>{
        ReceiverWrapper{
            receiver
        }
    }
}

impl<T: Send> Future for ReceiverWrapper<T>{
    type Item  = T;
    type Error = TryRecvError; // only returned on Disconnected
    fn poll(&mut self) -> Result<Async<Self::Item>, Self::Error>{
        match self.receiver.try_recv() {
            Ok(k)  => Ok(Async::Ready(k)),
            Err(e) =>
                if let TryRecvError::Empty = e{
                    Ok(Async::NotReady)
                }
                else{
                    Err(e) // This is a TryRecvError::Disconnected
                }
        }
    }
}
