use anyhow::Result;
use once_map::OnceMap;
use r3solvr::{CachedResolver, Symbol, SymbolResolver};
use std::sync::LazyLock;

static SYSTEM_LIBRARY_RESOLVER: LazyLock<SystemLibraryResolver> =
    LazyLock::new(SystemLibraryResolver::new);

pub struct SystemLibraryResolver {
    resolvers: OnceMap<String, CachedResolver>,
}

impl SystemLibraryResolver {
    fn new() -> Self {
        Self {
            resolvers: OnceMap::new(),
        }
    }

    pub fn resolve(&self, library_name: &str, symbol_name: &str) -> Result<Symbol> {
        Ok(self.resolvers.map_try_insert(
            library_name.into(),
            |name| CachedResolver::from_file(format!("/system/lib64/{name}")),
            |_, v| v.lookup_symbol(symbol_name),
        )??)
    }

    pub fn instance() -> &'static Self {
        &SYSTEM_LIBRARY_RESOLVER
    }
}
