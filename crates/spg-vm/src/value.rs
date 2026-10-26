//! Runtime values the VM works on. not the same as `spg_typecheck::Ty`,
//! which is a compile-time type. `Value` is what actually lives on the
//! stack and in slots while the program runs. the real stuff.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

/// `Rc<RefCell<>>` for arrays: cheap clone (shared handle) plus
/// interior mutability, matching the single-thread model we use for
/// now. `Arc<Mutex<>>` is the plan once real threads show up. (lol, threads.)
pub type ArrayRef = Rc<RefCell<Vec<Value>>>;

/// what a map hashes on. only int, string, bool and enum can be keys
/// (the typechecker rejects everything else), so building one from any
/// other `Value` is a compiler bug, and `from_value` returns `None`. blame the compiler.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MapKey {
    Int(i64),
    Str(Rc<String>),
    Bool(bool),
    Enum(u32, u32),
}

impl MapKey {
    pub fn from_value(v: &Value) -> Option<MapKey> {
        match v {
            Value::Int(n) => Some(MapKey::Int(*n)),
            Value::String(s) => Some(MapKey::Str(s.clone())),
            Value::Bool(b) => Some(MapKey::Bool(*b)),
            Value::Enum(d, i) => Some(MapKey::Enum(*d, *i)),
            _ => None,
        }
    }
}

/// an insertion-ordered hash map: `entries` keeps the order keys were first
/// inserted (that's the order `@map.keys`, `@map.values` and `print` show),
/// `index` finds an entry by key. overwriting keeps its
/// position. `remove` is O(n): it shifts everything after it. ouch, but fine.
/// lookups, inserts and overwrites are O(1). the good part.
#[derive(Debug, Default)]
pub struct SpgMap {
    index: HashMap<MapKey, usize>,
    entries: Vec<(Value, Value)>,
}

impl SpgMap {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, key: &Value) -> Option<&Value> {
        let pos = *self.index.get(&MapKey::from_value(key)?)?;
        Some(&self.entries[pos].1)
    }

    pub fn contains(&self, key: &Value) -> bool {
        self.get(key).is_some()
    }

    /// inserts or overwrites; `false` only if `key` cannot be a key (compiler bug)
    pub fn insert(&mut self, key: Value, value: Value) -> bool {
        let Some(k) = MapKey::from_value(&key) else { return false };
        match self.index.get(&k) {
            Some(&pos) => self.entries[pos].1 = value,
            None => {
                self.index.insert(k, self.entries.len());
                self.entries.push((key, value));
            }
        }
        true
    }

    /// removes `key`; `true` if it was there
    pub fn remove(&mut self, key: &Value) -> bool {
        let Some(k) = MapKey::from_value(key) else { return false };
        let Some(pos) = self.index.remove(&k) else { return false };
        self.entries.remove(pos);
        for slot in self.index.values_mut() {
            if *slot > pos {
                *slot -= 1;
            }
        }
        true
    }

    pub fn entries(&self) -> &[(Value, Value)] {
        &self.entries
    }
}

/// maps share by handle, like arrays: `let b = a` and passing to a function
/// both see the same map. spooky, but intended.
pub type MapRef = Rc<RefCell<SpgMap>>;

/// a struct's field storage: positional (`Vec<Value>`, indexed by
/// the slot `spg-compiler` resolved from the field name at compile
/// time. see `OpCode::FieldGet`/`FieldSet`) not name-keyed, so
/// field access is a plain index, no hash lookup. `def` carries
/// the struct's name and its field names (same order as `values`)
/// purely for `Display` and debugging. nothing at runtime looks a field up
/// by name through it. ever.
#[derive(Debug)]
pub struct StructDef {
    pub name: String,
    pub field_names: Vec<String>,
    /// native kind of each field (`None` for fields native can't read or write)
    pub field_kinds: Vec<Option<crate::chunk::NativeKind>>,
}
pub type StructRef = Rc<RefCell<(Rc<StructDef>, Vec<Value>)>>;

/// an enum type: its name and its variants in declaration order. a variant's
/// position is its identity at runtime (`Value::Enum(def, index)`).
#[derive(Debug)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<String>,
    /// each variant's value (`c.value`), same order as `variants`. nothing fancy.
    pub values: Vec<Value>,
}

thread_local! {
    /// every enum of the running program, indexed by `Value::Enum`'s first
    /// field. `Display` can't reach the VM, and an enum must print
    /// its variant name even nested in an array or struct, so the
    /// table lives here. `Vm::new` fills it (one thread, so no races. yet.)
    static ENUM_DEFS: RefCell<Vec<Rc<EnumDef>>> = RefCell::new(Vec::new());
}

/// registers the program's enums for printing (`Vm::new` calls this)
pub fn install_enum_defs(defs: &[Rc<EnumDef>]) {
    ENUM_DEFS.with(|d| *d.borrow_mut() = defs.to_vec());
}

/// the declared name of the enum at `index` (the first field of `Value::Enum`), if the program has it
pub fn enum_name(index: u32) -> Option<String> {
    ENUM_DEFS.with(|defs| defs.borrow().get(index as usize).map(|d| d.name.clone()))
}

/// the value of `variant` in the enum named `enum_name`, if the running program declares both
pub fn enum_value_by_name(enum_name: &str, variant: &str) -> Option<Value> {
    ENUM_DEFS.with(|defs| {
        let defs = defs.borrow();
        let index = defs.iter().position(|d| d.name == enum_name)?;
        let variant_index = defs[index].variants.iter().position(|v| v == variant)?;
        Some(Value::Enum(index as u32, variant_index as u32))
    })
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(Rc<String>),
    Array(ArrayRef),
    Map(MapRef),
    Struct(StructRef),
    /// `(enum index, variant index)`. see `EnumDef`. 8 bytes, so `Value` doesn't
    /// grow, and it's as cheap to copy as an `Int`
    Enum(u32, u32),
    /// a function value: a function (named or lambda) plus the values its
    /// lambda captured. shared like arrays, so copying copies a pointer
    Func(Rc<Closure>),
    /// a `task<T>`: the outcome of a `-> task<T>` call. see `TaskCell`
    Task(Rc<TaskCell>),
    /// `none`: the absent value of an optional type (`T?`). nothing here, nowhere.
    Nil,
    Void,
}

/// the outcome of a `-> task<T>` call. a task starts at once (the body runs until it
/// blocks or ends), and `.wait()` reads the outcome. a failed task re-throws its
/// error (and trace) there.
#[derive(Debug)]
pub struct TaskCell {
    /// the task's thread id (see `sched`). waiting on it means waiting for this id
    pub id: u64,
    /// `None` while the task is still running (or blocked). set once, right before it finishes
    pub outcome: RefCell<Option<TaskOutcome>>,
    /// has anyone called `.wait()` (or `@task.all`) on it. a failed task nobody waited on gets a warning
    pub waited: std::cell::Cell<bool>,
}

#[derive(Debug, Clone)]
pub enum TaskOutcome {
    Done(Value),
    /// the error, and the trace it had when the task body gave up on it
    Failed(crate::error::RuntimeError, Vec<crate::vm::TraceFrame>, usize),
}

/// the code of a function value: an index into `Program::functions`, and the
/// captured values that come before the args when it's called
#[derive(Debug)]
pub struct Closure {
    pub index: usize,
    pub captures: Vec<Value>,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Map(_) => "map",
            Value::Struct(_) => "struct",
            Value::Enum(..) => "enum",
            Value::Func(_) => "function",
            Value::Task(_) => "task",
            Value::Void => "void",
            Value::Nil => "none",
        }
    }

    /// what the builtin `type(x)` gives: the basic names, and the declared name of a struct or
    /// an enum (so two structs can be told apart). `void` is internal and is never shown to a program
    pub fn type_label(&self) -> String {
        match self {
            Value::Struct(s) => s.borrow().0.name.clone(),
            Value::Enum(def, _) => enum_name(*def).unwrap_or_else(|| "enum".to_string()),
            other => other.type_name().to_string(),
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            // the typechecker guarantees conditions are Bool before this
            // ever runs, so another variant here is a compiler bug. blame the compiler, again.
            other => panic!("VM bug: expected bool, got {}", other.type_name()),
        }
    }

    /// structural equality for builtins like `contains`/`indexOf`. not a
    /// derived `PartialEq` because `Array` holds `Rc<RefCell<Vec<Value>>>`,
    /// and arrays need their CONTENTS compared, not pointers,
    /// or a borrow-hostile derive. not used for the `==`
    /// operator itself (typechecked separately, same-type pairs only).
    pub fn structurally_equal(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Void, Value::Void) => true,
            (Value::Nil, Value::Nil) => true,
            (Value::Array(a), Value::Array(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.structurally_equal(y))
            }
            (Value::Map(a), Value::Map(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                // same keys and equal values. insertion order doesn't matter. (I checked.)
                a.len() == b.len()
                    && a.entries().iter().all(|(k, v)| b.get(k).map_or(false, |w| v.structurally_equal(w)))
            }
            (Value::Struct(a), Value::Struct(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                // same struct type and every field equal. field names
                // skipped, because same-type structs always
                // have identical field names in identical order. trust.
                a.0.name == b.0.name
                    && a.1.len() == b.1.len()
                    && a.1.iter().zip(b.1.iter()).all(|(x, y)| x.structurally_equal(y))
            }
            (Value::Enum(d1, v1), Value::Enum(d2, v2)) => d1 == d2 && v1 == v2,
            _ => false,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", format_float(*n)),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Map(m) => {
                let m = m.borrow();
                if m.len() == 0 {
                    return write!(f, "[:]");
                }
                write!(f, "[")?;
                for (i, (k, v)) in m.entries().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "]")
            }
            Value::Void => write!(f, "void"),
            Value::Nil => write!(f, "none"),
            Value::Func(_) => write!(f, "<function>"),
            Value::Task(_) => write!(f, "<task>"),
            Value::Enum(def, variant) => ENUM_DEFS.with(|d| {
                match d.borrow().get(*def as usize).and_then(|e| e.variants.get(*variant as usize)) {
                    Some(name) => write!(f, "{}", name),
                    None => write!(f, "<enum {}.{}>", def, variant),
                }
            }),
            Value::Struct(s) => {
                let (def, values) = &*s.borrow();
                write!(f, "{} {{ ", def.name)?;
                for (i, (name, value)) in def.field_names.iter().zip(values.iter()).enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, value)?;
                }
                write!(f, " }}")
            }
        }
    }
}

/// how a float shows up on screen. Rust prints `2.0` as `2`, which looks
/// like an int, so a whole float keeps its `.0` (`-0.0` included). floats, man.
pub fn format_float(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 {
        if n == 0.0 && n.is_sign_negative() {
            return "-0.0".to_string();
        }
        return format!("{:.1}", n);
    }
    format!("{}", n)
}
