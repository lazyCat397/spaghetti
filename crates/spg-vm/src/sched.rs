//! the baton: which thread may run spg code right now. every task is a real OS
//! thread, but only the thread holding the baton touches the VM. a task gives
//! the baton away only when it blocks (sleep, waiting on another task) or
//! finishes, so the order of prints is always the same. no races. no surprises.

use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard};

/// nobody gets the baton again. set when the program ends, so the tasks still
/// blocked in the background just stay asleep until the process goes away.
const SHUTDOWN: u64 = u64::MAX;

#[derive(Default)]
struct State {
    /// who holds the baton. `None` when every thread is blocked (sleeping, say)
    running: Option<u64>,
    /// threads that want the baton back, in line order
    ready: VecDeque<u64>,
    /// tasks that finished. a waiter checks this, not the task itself
    finished: HashSet<u64>,
    next_id: u64,
}

pub struct Sched {
    state: Mutex<State>,
    turn: Condvar,
}

thread_local! {
    /// the id of the thread asking. the main program is 0, always.
    static ME: Cell<u64> = const { Cell::new(0) };
}

/// carries a value into a task's thread. only sound because of the baton:
/// nothing else touches the value at the same time. that's the whole deal.
pub struct Unsend<T>(pub T);
// SAFETY: see above. the baton is the lock, the Rc counts are never touched in parallel.
unsafe impl<T> Send for Unsend<T> {}

impl<T> Unsend<T> {
    /// takes the value out. a method call, so the closure captures the whole wrapper.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl Sched {
    /// the main program starts holding the baton. it's thread 0. obviously.
    pub fn new() -> Self {
        Sched {
            state: Mutex::new(State { running: Some(0), next_id: 1, ..State::default() }),
            turn: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // a panicked thread shouldn't take the whole scheduler down with it
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pass_on(st: &mut State) {
        st.running = st.ready.pop_front();
    }

    /// who am I? the thread's own id.
    pub fn current() -> u64 {
        ME.with(|m| m.get())
    }

    /// called by a new task thread once, before it does anything.
    pub fn set_current(id: u64) {
        ME.with(|m| m.set(id));
    }

    /// reserves an id for a task that's about to get its thread.
    pub fn new_id(&self) -> u64 {
        let mut st = self.lock();
        let id = st.next_id;
        st.next_id += 1;
        id
    }

    /// the creator goes to the front of the line, and the new task takes the baton.
    /// the creator then has to call `wait_turn`, and waits until the task blocks or ends.
    pub fn handoff(&self, creator: u64, task: u64) {
        let mut st = self.lock();
        st.ready.push_front(creator);
        st.running = Some(task);
        self.turn.notify_all();
    }

    /// sleeps until this thread holds the baton. nothing runs before that.
    pub fn wait_turn(&self, me: u64) {
        let mut st = self.lock();
        while st.running != Some(me) {
            st = self.turn.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// about to block: give the baton to whoever's next in line.
    pub fn release(&self) {
        let mut st = self.lock();
        Self::pass_on(&mut st);
        self.turn.notify_all();
    }

    /// about to wait on a task: give the baton away, and sleep until that task is done.
    pub fn release_until_finished(&self, task: u64) {
        let mut st = self.lock();
        Self::pass_on(&mut st);
        self.turn.notify_all();
        while !st.finished.contains(&task) {
            st = self.turn.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// done blocking: take the baton back. free right now? take it. otherwise, wait in line.
    pub fn reacquire(&self, me: u64) {
        let mut st = self.lock();
        if st.running.is_none() {
            st.running = Some(me);
            return;
        }
        st.ready.push_back(me);
        while st.running != Some(me) {
            st = self.turn.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// the task is over. its outcome is already stored. pass the baton on.
    pub fn finish(&self, task: u64) {
        let mut st = self.lock();
        st.finished.insert(task);
        Self::pass_on(&mut st);
        self.turn.notify_all();
    }

    /// the program is over: nothing else gets to run. see SHUTDOWN.
    pub fn shutdown(&self) {
        let mut st = self.lock();
        st.ready.clear();
        st.running = Some(SHUTDOWN);
        self.turn.notify_all();
    }
}
