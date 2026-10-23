//! read-only values, for `const`, and the copies that make them usable.
//!
//! `const c = expr` copies `expr` deeply (arrays, maps and structs, all the way
//! down) and registers every container of the copy here. a write to a registered
//! container fails with `RuntimeError::ReadOnly`. the copy is never shared with a
//! mutable variable, so nothing mutable can reach it through an alias.
//!
//! `.copy()` makes the same deep copy, but nothing is registered: the result is a
//! plain mutable value.
//!
//! the table is keyed by the container's address. each entry holds a `Weak`, so the
//! allocation can't be freed (and its address reused) while the entry exists. dead
//! entries are pruned whenever a new const is made. until the first const, the
//! checks are one relaxed atomic load: programs without `const` pay nothing.
//!
//! threads: each `task` runs on its own thread, but only the one holding the baton
//! (see `sched`) runs VM code, so the table is never touched concurrently. the
//! `Mutex` is only there so the compiler accepts a static. it's uncontended.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::error::RuntimeError;
use crate::value::{ArrayRef, MapRef, SpgMap, StructDef, StructRef, Value};

enum Entry {
    Arr(Weak<RefCell<Vec<Value>>>),
    Map(Weak<RefCell<SpgMap>>),
    Rec(Weak<RefCell<(Rc<StructDef>, Vec<Value>)>>),
}

impl Entry {
    fn alive(&self) -> bool {
        match self {
            Entry::Arr(w) => w.strong_count() > 0,
            Entry::Map(w) => w.strong_count() > 0,
            Entry::Rec(w) => w.strong_count() > 0,
        }
    }
}

struct Table(HashMap<usize, Entry>);
// SAFETY: `Rc`/`Weak` aren't `Send`, but VM code only ever runs on the thread holding the baton
unsafe impl Send for Table {}

static ANY: AtomicBool = AtomicBool::new(false);
static TABLE: Mutex<Option<Table>> = Mutex::new(None);

fn with_table<R>(f: impl FnOnce(&mut HashMap<usize, Entry>) -> R) -> R {
    let mut guard = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let table = guard.get_or_insert_with(|| Table(HashMap::new()));
    f(&mut table.0)
}

fn register(addr: usize, entry: Entry) {
    ANY.store(true, Ordering::Relaxed);
    with_table(|t| {
        t.insert(addr, entry);
    });
}

fn prune() {
    with_table(|t| t.retain(|_, e| e.alive()));
}

/// `true` when the container at `addr` is read-only. the JIT asks this too.
#[inline]
pub fn is_frozen_addr(addr: usize) -> bool {
    ANY.load(Ordering::Relaxed) && with_table(|t| t.contains_key(&addr))
}

#[inline]
fn guard(addr: usize) -> Result<(), RuntimeError> {
    if is_frozen_addr(addr) {
        Err(RuntimeError::ReadOnly)
    } else {
        Ok(())
    }
}

/// fails when writing to this array would modify a const value
#[inline]
pub fn check_array(a: &ArrayRef) -> Result<(), RuntimeError> {
    guard(Rc::as_ptr(a) as *const () as usize)
}

/// fails when writing to this map would modify a const value
#[inline]
pub fn check_map(m: &MapRef) -> Result<(), RuntimeError> {
    guard(Rc::as_ptr(m) as *const () as usize)
}

/// fails when writing to this struct would modify a const value
#[inline]
pub fn check_struct(s: &StructRef) -> Result<(), RuntimeError> {
    guard(Rc::as_ptr(s) as *const () as usize)
}

/// the deep, read-only copy that `const` stores: a fresh copy of every container
/// reachable from `v`, each one registered as read-only. scalars and functions are
/// copied as they are.
pub fn copy_frozen(v: &Value) -> Value {
    prune();
    let mut memo = HashMap::new();
    copy_value(v, &mut memo, true)
}

/// `x.copy()`: the same deep copy, but mutable. nothing is registered.
pub fn copy_mutable(v: &Value) -> Value {
    let mut memo = HashMap::new();
    copy_value(v, &mut memo, false)
}

/// records a new container of a copy, if that copy is read-only
fn reg(freeze: bool, addr: usize, entry: Entry) {
    if freeze {
        register(addr, entry);
    }
}

fn copy_value(v: &Value, memo: &mut HashMap<usize, Value>, freeze: bool) -> Value {
    match v {
        Value::Array(a) => {
            let key = Rc::as_ptr(a) as *const () as usize;
            if let Some(done) = memo.get(&key) {
                return done.clone();
            }
            // registered before filling, and memoized before recursing: cycles come back to this copy
            let new: ArrayRef = Rc::new(RefCell::new(Vec::new()));
            memo.insert(key, Value::Array(new.clone()));
            reg(freeze, Rc::as_ptr(&new) as *const () as usize, Entry::Arr(Rc::downgrade(&new)));
            let items: Vec<Value> = a.borrow().iter().map(|x| copy_value(x, memo, freeze)).collect();
            *new.borrow_mut() = items;
            Value::Array(new)
        }
        Value::Map(m) => {
            let key = Rc::as_ptr(m) as *const () as usize;
            if let Some(done) = memo.get(&key) {
                return done.clone();
            }
            let new: MapRef = Rc::new(RefCell::new(SpgMap::default()));
            memo.insert(key, Value::Map(new.clone()));
            reg(freeze, Rc::as_ptr(&new) as *const () as usize, Entry::Map(Rc::downgrade(&new)));
            let pairs: Vec<(Value, Value)> = m.borrow().entries().to_vec();
            for (k, val) in pairs {
                let copied = copy_value(&val, memo, freeze);
                let _ = new.borrow_mut().insert(k, copied); // keys were valid in the original, so they're valid here
            }
            Value::Map(new)
        }
        Value::Struct(s) => {
            let key = Rc::as_ptr(s) as *const () as usize;
            if let Some(done) = memo.get(&key) {
                return done.clone();
            }
            let (def, originals) = {
                let b = s.borrow();
                (b.0.clone(), b.1.clone())
            };
            let new: StructRef = Rc::new(RefCell::new((def, Vec::new())));
            memo.insert(key, Value::Struct(new.clone()));
            reg(freeze, Rc::as_ptr(&new) as *const () as usize, Entry::Rec(Rc::downgrade(&new)));
            let fields: Vec<Value> = originals.iter().map(|x| copy_value(x, memo, freeze)).collect();
            new.borrow_mut().1 = fields;
            Value::Struct(new)
        }
        other => other.clone(),
    }
}
