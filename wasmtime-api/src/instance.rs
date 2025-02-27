use crate::context::Context;
use crate::externals::Extern;
use crate::module::Module;
use crate::runtime::Store;
use failure::Error;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use wasmtime_jit::{instantiate, Resolver};
use wasmtime_runtime::{Export, InstanceHandle};

struct SimpleResolver {
    imports: Vec<(String, String, Rc<RefCell<Extern>>)>,
}

impl Resolver for SimpleResolver {
    fn resolve(&mut self, name: &str, field: &str) -> Option<Export> {
        // TODO speedup lookup
        self.imports
            .iter()
            .find(|(n, f, _)| name == n && field == f)
            .map(|(_, _, e)| e.borrow_mut().get_wasmtime_export())
    }
}

pub fn instantiate_in_context(
    data: &[u8],
    imports: Vec<(String, String, Rc<RefCell<Extern>>)>,
    mut context: Context,
    exports: Rc<RefCell<HashMap<String, Option<wasmtime_runtime::Export>>>>,
) -> Result<(InstanceHandle, HashSet<Context>), Error> {
    let mut contexts = HashSet::new();
    let debug_info = context.debug_info();
    let mut resolver = SimpleResolver { imports };
    let instance = instantiate(
        &mut context.compiler(),
        data,
        &mut resolver,
        exports,
        debug_info,
    )?;
    contexts.insert(context);
    Ok((instance, contexts))
}

#[derive(Clone)]
pub struct Instance {
    instance_handle: InstanceHandle,

    // We need to keep CodeMemory alive.
    contexts: HashSet<Context>,

    exports: Box<[Rc<RefCell<Extern>>]>,
}

impl Instance {
    pub fn new(
        store: Rc<RefCell<Store>>,
        module: Rc<RefCell<Module>>,
        externs: &[Rc<RefCell<Extern>>],
    ) -> Result<Instance, Error> {
        let context = store.borrow_mut().context().clone();
        let exports = store.borrow_mut().global_exports().clone();
        let imports = module
            .borrow()
            .imports()
            .iter()
            .zip(externs.iter())
            .map(|(i, e)| (i.module().to_string(), i.name().to_string(), e.clone()))
            .collect::<Vec<_>>();
        let (mut instance_handle, contexts) =
            instantiate_in_context(module.borrow().binary(), imports, context, exports)?;

        let exports = {
            let module = module.borrow();
            let mut exports = Vec::with_capacity(module.exports().len());
            for export in module.exports() {
                let name = export.name().to_string();
                let export = instance_handle.lookup(&name).expect("export");
                exports.push(Rc::new(RefCell::new(Extern::from_wasmtime_export(
                    store.clone(),
                    instance_handle.clone(),
                    export,
                ))));
            }
            exports.into_boxed_slice()
        };
        Ok(Instance {
            instance_handle,
            contexts,
            exports,
        })
    }

    pub fn exports(&self) -> &[Rc<RefCell<Extern>>] {
        &self.exports
    }

    pub fn from_handle(
        store: Rc<RefCell<Store>>,
        instance_handle: InstanceHandle,
    ) -> Result<(Instance, HashMap<String, usize>), Error> {
        let contexts = HashSet::new();

        let mut exports = Vec::new();
        let mut export_names_map = HashMap::new();
        let mut mutable = instance_handle.clone();
        for (name, _) in instance_handle.clone().exports() {
            let export = mutable.lookup(name).expect("export");
            export_names_map.insert(name.to_owned(), exports.len());
            exports.push(Rc::new(RefCell::new(Extern::from_wasmtime_export(
                store.clone(),
                instance_handle.clone(),
                export.clone(),
            ))));
        }

        Ok((
            Instance {
                instance_handle,
                contexts,
                exports: exports.into_boxed_slice(),
            },
            export_names_map,
        ))
    }

    pub fn handle(&self) -> &InstanceHandle {
        &self.instance_handle
    }

    pub fn get_wasmtime_memory(&self) -> Option<wasmtime_runtime::Export> {
        let mut instance_handle = self.instance_handle.clone();
        instance_handle.lookup("memory")
    }
}
