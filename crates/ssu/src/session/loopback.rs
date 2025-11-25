use std::{collections::VecDeque, sync::Mutex};

use crate::session::{SessionEndpoint, Ticked};

pub struct LoopbackSession {
    queue: Mutex<VecDeque<u8>>,
}

impl LoopbackSession {
    pub fn new(initial: String) -> Self {
        LoopbackSession {
            queue: VecDeque::from_iter(initial.bytes()).into(),
        }
    }
}

impl SessionEndpoint for LoopbackSession {
    fn recv(&mut self) -> Ticked {
        match self.queue.lock().unwrap().pop_front() {
            Some(b) => Ticked::Byte(b),
            None => Ticked::IdleInput,
        }
    }

    fn send(&mut self, b: u8) {
        self.queue.lock().unwrap().push_back(b);
    }
}
