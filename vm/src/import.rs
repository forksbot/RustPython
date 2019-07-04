/*
 * Import mechanics
 */

use crate::bytecode::CodeObject;
use crate::frame::Scope;
use crate::obj::{objcode, objsequence, objstr, objtype};
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;
#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler::compile;

pub fn init_importlib(vm: &VirtualMachine, external: bool) -> PyResult {
    let importlib = import_frozen(vm, "_frozen_importlib")?;
    let impmod = import_builtin(vm, "_imp")?;
    let install = vm.get_attribute(importlib.clone(), "_install")?;
    vm.invoke(install, vec![vm.sys_module.clone(), impmod])?;
    vm.import_func
        .replace(vm.get_attribute(importlib.clone(), "__import__")?);
    if external && cfg!(feature = "rustpython-compiler") {
        let install_external =
            vm.get_attribute(importlib.clone(), "_install_external_importers")?;
        vm.invoke(install_external, vec![])?;
    }
    Ok(vm.get_none())
}

pub fn import_frozen(vm: &VirtualMachine, module_name: &str) -> PyResult {
    vm.frozen
        .borrow()
        .get(module_name)
        .ok_or_else(|| vm.new_import_error(format!("Cannot import frozen module {}", module_name)))
        .and_then(|frozen| import_codeobj(vm, module_name, frozen.clone(), false))
}

pub fn import_builtin(vm: &VirtualMachine, module_name: &str) -> PyResult {
    vm.stdlib_inits
        .borrow()
        .get(module_name)
        .ok_or_else(|| vm.new_import_error(format!("Cannot import bultin module {}", module_name)))
        .and_then(|make_module_func| {
            let module = make_module_func(vm);
            let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules")?;
            sys_modules.set_item(module_name, module.clone(), vm)?;
            Ok(module)
        })
}

#[cfg(feature = "rustpython-compiler")]
pub fn import_file(
    vm: &VirtualMachine,
    module_name: &str,
    file_path: String,
    content: String,
) -> PyResult {
    let code_obj = compile::compile(&content, &compile::Mode::Exec, file_path)
        .map_err(|err| vm.new_syntax_error(&err))?;
    import_codeobj(vm, module_name, code_obj, true)
}

pub fn import_codeobj(
    vm: &VirtualMachine,
    module_name: &str,
    code_obj: CodeObject,
    set_file_attr: bool,
) -> PyResult {
    let attrs = vm.ctx.new_dict();
    attrs.set_item("__name__", vm.new_str(module_name.to_string()), vm)?;
    if set_file_attr {
        attrs.set_item("__file__", vm.new_str(code_obj.source_path.to_owned()), vm)?;
    }
    let module = vm.ctx.new_module(module_name, attrs.clone());

    // Store module in cache to prevent infinite loop with mutual importing libs:
    let sys_modules = vm.get_attribute(vm.sys_module.clone(), "modules")?;
    sys_modules.set_item(module_name, module.clone(), vm)?;

    // Execute main code in module:
    vm.run_code_obj(
        objcode::PyCode::new(code_obj).into_ref(vm),
        Scope::with_builtins(None, attrs, vm),
    )?;
    Ok(module)
}

pub fn remove_importlib_frames(vm: &VirtualMachine, exc: &PyObjectRef) -> PyObjectRef {
    let always_trim = objtype::isinstance(exc, &vm.ctx.exceptions.import_error);

    if let Ok(tb) = vm.get_attribute(exc.clone(), "__traceback__") {
        if objtype::isinstance(&tb, &vm.ctx.list_type()) {
            let mut tb_entries = objsequence::get_elements_list(&tb).to_vec();
            tb_entries.reverse();
            let mut new_tb = Vec::with_capacity(tb_entries.len());

            for tb_entry in tb_entries.iter() {
                let mut current_chunk = vec![];
                let location_attrs = objsequence::get_elements_tuple(&tb_entry);
                let file_name = objstr::get_value(&location_attrs[0]);
                if file_name != "_frozen_importlib" && file_name != "_frozen_importlib_external" {
                    current_chunk.clear();
                    new_tb.push(tb_entry.clone())
                } else {
                    current_chunk.push(tb_entry.clone());
                    let run_obj_name = objstr::get_value(&location_attrs[0]);
                    if run_obj_name == "_call_with_frames_removed" || always_trim {
                        new_tb.append(&mut current_chunk);
                    }
                };
            }
            new_tb.reverse();
            vm.set_attr(exc, "__traceback__", vm.ctx.new_list(new_tb))
                .unwrap();
        }
    }
    exc.clone()
}
