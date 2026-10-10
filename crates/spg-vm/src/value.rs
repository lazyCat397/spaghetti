//! Runtime values the VM operates on. Distinct from `spg_typecheck::Ty`,
//! which is a compile-time type — `Value` is what actually lives on the
//! stack / in slots while the program executes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

/// `Rc<RefCell<>>` for arrays: a cheap clone (shared handle) plus
/// interior mutability, matching the single-thread model I'm using for
/// now — `Arc<Mutex<>>` is the planned upgrade once real threads land.
pub type ArrayRef = Rc<RefCell<Vec<Value>>>;

/// What a map hashes on. Only int, string, bool and enum values can be keys
/// (the typechecker rejects every other key type), so building one from any
/// other `Value` is a compiler bug and `from_value` returns `None`.
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

/// An insertion-ordered hash map: `entries` keeps the order keys were first
/// inserted (that is the order `@map.keys`, `@map.values` and `print` show),
/// `index` finds an entry's position by key. Overwriting a key keeps its
/// position. `remove` is O(n) (it shifts the entries after it and fixes their
/// positions); lookups, inserts and overwrites are O(1).
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

/// maps share by handle like arrays: `let b = a` and passing to a function
/// both see the same map
pub type MapRef = Rc<RefCell<SpgMap>>;

/// A struct value's field storage: positional (`Vec<Value>`, indexed by
/// the slot `spg-compiler` resolved from the field name at compile
/// time — see `OpCode::FieldGet`/`FieldSet`) rather than name-keyed, so
/// field access is a plain index instead of a hash lookup. `def` carries
/// the struct's name and its field names (in the same order as `values`)
/// purely for `Display`/debugging — nothing at runtime looks a field up
/// by name through it.
#[derive(Debug)]
pub struct StructDef {
    pub name: String,
    pub field_names: Vec<String>,
}
pub type StructRef = Rc<RefCell<(Rc<StructDef>, Vec<Value>)>>;

/// An enum type: its name and its variants in declaration order. A variant's
/// position is its identity at run time (`Value::Enum(def, index)`).
#[derive(Debug)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<String>,
    /// each variant's value (`c.value`), same order as `variants`
    pub values: Vec<Value>,
}

thread_local! {
    /// every enum of the running program, indexed by `Value::Enum`'s first
    /// field. `Display` has no access to the VM, and an enum value must print
    /// its variant's name even nested inside an array or a struct, so the
    /// table lives here; `Vm::new` fills it (the VM runs on one thread).
    static ENUM_DEFS: RefCell<Vec<Rc<EnumDef>>> = RefCell::new(Vec::new());
}

/// registers the program's enums for printing (called by `Vm::new`)
pub fn install_enum_defs(defs: &[Rc<EnumDef>]) {
    ENUM_DEFS.with(|d| *d.borrow_mut() = defs.to_vec());
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
    /// `(enum index, variant index)` — see `EnumDef`. 8 bytes, so `Value` does
    /// not grow, and cheap to copy like an `Int`
    Enum(u32, u32),
    Void,
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
            Value::Void => "void",
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            // the typechecker guarantees conditions are Bool before this
            // ever runs, so reaching another variant here is a compiler bug
            other => panic!("VM bug: expected bool, got {}", other.type_name()),
        }
    }

    /// Structural equality for builtins like `contains`/`indexOf` — not a
    /// derived `PartialEq` because `Array` holds `Rc<RefCell<Vec<Value>>>`
    /// and comparing arrays needs to compare contents, not pointer
    /// identity or a borrow-unfriendly derive. Not used for the `==`
    /// operator itself (typechecked separately, same-type pairs only).
    pub fn structurally_equal(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Void, Value::Void) => true,
            (Value::Array(a), Value::Array(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.structurally_equal(y))
            }
            (Value::Map(a), Value::Map(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                // same keys and equal values, insertion order does not matter
                a.len() == b.len()
                    && a.entries().iter().all(|(k, v)| b.get(k).map_or(false, |w| v.structurally_equal(w)))
            }
            (Value::Struct(a), Value::Struct(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                // same struct type and every field equal, field name
                // comparison skipped since same-type structs always
                // have identical field_names in identical order
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
            Value::Float(n) => write!(f, "{}", n),
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
