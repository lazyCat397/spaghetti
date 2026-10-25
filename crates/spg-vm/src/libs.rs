//! the boundary with the native libraries (`spg-libs`): the VM's values go in as plain
//! `LibValue`s, the answer comes back and becomes a VM value again. a library never sees
//! the VM, and the VM never knows what a library does. that's the whole deal.

use crate::value::Closure;
use std::cell::RefCell;
use std::rc::Rc;

use spg_libs::{CallCtx, LibValue};

use crate::chunk::Program;
use crate::error::RuntimeError;
use crate::value::{SpgMap, Value};

/// how deep a value is followed before it counts as "contains itself"
const MAX_DEPTH: usize = 512;

/// runs function number `index` of the library registry. `caller_file` is the script that
/// made the call (a library may resolve relative paths from its folder).
pub fn call(
    program: &Program,
    index: u16,
    args: Vec<Value>,
    handlers: &mut Vec<Rc<Closure>>,
    caller_file: Option<&str>,
    caller_line: u32,
) -> Result<Value, RuntimeError> {
    let (_, function) = spg_libs::function(index)
        .ok_or_else(|| RuntimeError::Process { function: "lib", reason: format!("internal: no library function #{index}") })?;
    // a function passed straight to the library becomes a handler: the VM keeps the closure and
    // the library keeps its number (see `spg_libs::queue_handler`)
    let args: Vec<LibValue> = args
        .into_iter()
        .map(|v| match v {
            Value::Func(closure) => {
                handlers.push(closure);
                LibValue::Handler((handlers.len() - 1) as u64)
            }
            other => to_lib(other, 0),
        })
        .collect();
    let answer = (function.call)(&CallCtx { caller_file, caller_line }, args).map_err(|e| match e.path {
        Some(path) => RuntimeError::Io { function: e.function, path, reason: e.reason },
        None => RuntimeError::Process { function: e.function, reason: e.reason },
    })?;
    from_lib(program, answer, function.name)
}

/// a VM value as a plain one. never fails: what a library can not look inside becomes `Opaque`
fn to_lib(value: Value, depth: usize) -> LibValue {
    if depth > MAX_DEPTH {
        return LibValue::Opaque("deep");
    }
    match value {
        Value::Nil => LibValue::Nil,
        Value::Void => LibValue::Void,
        Value::Int(n) => LibValue::Int(n),
        Value::Float(x) => LibValue::Float(x),
        Value::Bool(b) => LibValue::Bool(b),
        Value::String(s) => LibValue::Str(s.to_string()),
        Value::Array(items) => LibValue::Array(items.borrow().iter().cloned().map(|v| to_lib(v, depth + 1)).collect()),
        Value::Map(map) => LibValue::Map(
            map.borrow().entries().iter().map(|(k, v)| (to_lib(k.clone(), depth + 1), to_lib(v.clone(), depth + 1))).collect(),
        ),
        Value::Struct(s) => {
            let guard = s.borrow();
            let (def, values) = &*guard;
            LibValue::Struct {
                name: def.name.to_string(),
                fields: def.field_names.iter().zip(values.iter()).map(|(n, v)| (n.to_string(), to_lib(v.clone(), depth + 1))).collect(),
            }
        }
        Value::Enum(..) => LibValue::Enum(value.to_string()),
        Value::Func(_) => LibValue::Opaque("function"),
        Value::Task(_) => LibValue::Opaque("task"),
    }
}

fn from_lib(program: &Program, value: LibValue, function: &'static str) -> Result<Value, RuntimeError> {
    Ok(match value {
        LibValue::Void => Value::Void,
        LibValue::Nil => Value::Nil,
        LibValue::Int(n) => Value::Int(n),
        LibValue::Float(x) => Value::Float(x),
        LibValue::Bool(b) => Value::Bool(b),
        LibValue::Str(s) => Value::String(Rc::new(s)),
        LibValue::Array(items) => Value::Array(Rc::new(RefCell::new(
            items.into_iter().map(|i| from_lib(program, i, function)).collect::<Result<Vec<_>, _>>()?,
        ))),
        LibValue::Map(pairs) => {
            let mut map = SpgMap::default();
            for (k, v) in pairs {
                map.insert(from_lib(program, k, function)?, from_lib(program, v, function)?);
            }
            Value::Map(Rc::new(RefCell::new(map)))
        }
        LibValue::Struct { name, mut fields } => {
            let internal = |reason: String| RuntimeError::Process { function, reason };
            let def = program
                .struct_defs
                .iter()
                .find(|d| d.name == name)
                .cloned()
                .ok_or_else(|| internal(format!("{name} is not declared (internal error)")))?;
            // the library names its fields; the program's declaration decides the order
            let mut values = Vec::with_capacity(def.field_names.len());
            for field in def.field_names.iter() {
                let field = field.to_string();
                let at = fields
                    .iter()
                    .position(|(n, _)| *n == field)
                    .ok_or_else(|| internal(format!("{name} has no value for `{field}` (internal error)")))?;
                let (_, v) = fields.remove(at);
                values.push(from_lib(program, v, function)?);
            }
            Value::Struct(Rc::new(RefCell::new((def, values))))
        }
        LibValue::Variant(enum_name, variant) => crate::value::enum_value_by_name(&enum_name, &variant).ok_or_else(|| {
            RuntimeError::Process { function, reason: format!("internal: the program has no {enum_name}.{variant}") }
        })?,
        LibValue::Enum(_) | LibValue::Opaque(_) | LibValue::Handler(_) => {
            return Err(RuntimeError::Process { function, reason: "a library can not hand back this kind of value".to_string() })
        }
    })
}

/// a value a library hands back to a handler call (an argument of `queue_handler`)
pub fn from_handler_arg(program: &Program, value: LibValue) -> Result<Value, RuntimeError> {
    from_lib(program, value, "handler")
}
