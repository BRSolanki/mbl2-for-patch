use std::{borrow::Cow, collections::HashMap};

use plt_rs::DynamicLibrary;
use region::{protect_with_handle, Protection};

pub fn replace_plt_functions<const LEN: usize>(
    dyn_lib: &DynamicLibrary,
    functions: [(&str, *const u8); LEN],
) {
    let base_addr = dyn_lib.library().addr();
    let Some(table) = get_function_table(dyn_lib) else {
        log::warn!("Wtf");
        return;
    };
    for (fn_name, replacement) in functions {
        let name = Cow::Borrowed(fn_name);
        if let Some(fn_plt) = table.get(&name) {
            if let Err(error) = replace_plt_function(
                base_addr,
                fn_plt.r_offset as usize,
                replacement,
            ) {
                log::error!("Failed to replace {fn_name} PLT entry: {error}");
            }
        }
    }
}
fn replace_plt_function(
    base_addr: usize,
    offset: usize,
    replacement: *const u8,
) -> region::Result<()> {
    let plt_fn_ptr = (base_addr + offset) as *mut *const u8;
    const PTR_LEN: usize = std::mem::size_of::<usize>();
    unsafe {
        let _guard = protect_with_handle(plt_fn_ptr, PTR_LEN, Protection::READ_WRITE)?;
        // Replace the function address
        plt_fn_ptr.write_unaligned(replacement);
    }
    Ok(())
}

macro_rules! collect_entries {
    ($iter:ident,$syms:expr,$table:expr) => {
        $iter
            .entries()
            .iter()
            .map(|e| {
                $syms
                    .resolve_name(e.symbol_index() as usize, $table)
                    .map(|s| (s, e))
            })
            .flatten()
    };
}

pub fn get_function_table<'a>(
    dnlib: &'a DynamicLibrary,
) -> Option<HashMap<Cow<'a, str>, &'a plt_rs::elf64::DynRela>> {
    let string_table = dnlib.string_table();
    let symbols = dnlib.symbols()?;
    let mut hashmap = HashMap::new();
    if let Some(dyn_relas) = dnlib.addend_relocs() {
        let entries = collect_entries!(dyn_relas, &symbols, &string_table);
        hashmap.extend(entries);
    }
    if let Some(dyn_relas) = dnlib.plt_rela() {
        let entries = collect_entries!(dyn_relas, &symbols, &string_table);
        hashmap.extend(entries);
    }
    if hashmap.is_empty() {
        None
    } else {
        Some(hashmap)
    }
}
