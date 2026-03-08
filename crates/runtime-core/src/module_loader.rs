use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hasher;
use std::rc::Rc;
use std::sync::Arc;

use base64::Engine;
use deno_core::{
    error::ModuleLoaderError, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse,
    ModuleLoader, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType, ResolutionKind,
    SourceCodeCacheInfo,
};
use eszip::EszipV2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleCodeCacheEntry {
    pub hash: u64,
    pub data: Vec<u8>,
}

pub type ModuleCodeCacheMap = HashMap<String, ModuleCodeCacheEntry>;

fn module_source_hash(source: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(source);
    hasher.finish()
}

/// Module loader that resolves modules from an eszip bundle.
pub struct EszipModuleLoader {
    eszip: Arc<EszipV2>,
    inline_source_maps: bool,
    code_cache: Option<Rc<RefCell<ModuleCodeCacheMap>>>,
}

impl EszipModuleLoader {
    pub fn new(eszip: Arc<EszipV2>) -> Self {
        Self {
            eszip,
            inline_source_maps: true,
            code_cache: None,
        }
    }

    pub fn new_with_source_maps(eszip: Arc<EszipV2>, inline_source_maps: bool) -> Self {
        Self {
            eszip,
            inline_source_maps,
            code_cache: None,
        }
    }

    pub fn new_with_source_maps_and_code_cache(
        eszip: Arc<EszipV2>,
        inline_source_maps: bool,
        code_cache: Option<ModuleCodeCacheMap>,
    ) -> Self {
        Self {
            eszip,
            inline_source_maps,
            code_cache: Some(Rc::new(RefCell::new(code_cache.unwrap_or_default()))),
        }
    }

    pub fn code_cache_snapshot(&self) -> Option<ModuleCodeCacheMap> {
        self.code_cache.as_ref().map(|cache| cache.borrow().clone())
    }
}

impl ModuleLoader for EszipModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        deno_core::resolve_import(specifier, referrer).map_err(|e| {
            ModuleLoaderError::from(deno_error::JsErrorBox::generic(format!(
                "module resolution failed: {}",
                e
            )))
        })
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let specifier = module_specifier.clone();
        let eszip = self.eszip.clone();
        let inline_source_maps = self.inline_source_maps;
        let code_cache_store = self.code_cache.clone();

        ModuleLoadResponse::Async(Box::pin(async move {
            let module = eszip.get_module(specifier.as_str()).ok_or_else(|| {
                ModuleLoaderError::from(deno_error::JsErrorBox::generic(format!(
                    "module not found in eszip: {}",
                    specifier
                )))
            })?;

            let source = module.take_source().await.ok_or_else(|| {
                ModuleLoaderError::from(deno_error::JsErrorBox::generic(format!(
                    "module source unavailable: {}",
                    specifier
                )))
            })?;

            // eszip stores source maps separately. Attach them as inline
            // sourceMappingURL so debuggers can map transpiled JS back to TS.
            let mut source_bytes = source.to_vec();

            if inline_source_maps {
                if let Some(source_map) = module.take_source_map().await {
                    let has_mapping_url = source_bytes
                        .windows(b"sourceMappingURL".len())
                        .any(|w| w == b"sourceMappingURL");
                    if !source_map.is_empty() && !has_mapping_url {
                        let encoded_map =
                            base64::engine::general_purpose::STANDARD.encode(&*source_map);
                        let suffix = format!(
                            "\n//# sourceMappingURL=data:application/json;base64,{}\n",
                            encoded_map
                        );
                        source_bytes.extend_from_slice(suffix.as_bytes());
                    }
                }
            }

            let module_type = match module.kind {
                eszip::ModuleKind::JavaScript => ModuleType::JavaScript,
                eszip::ModuleKind::Json => ModuleType::Json,
                _ => ModuleType::JavaScript,
            };

            let computed_hash = module_source_hash(&source_bytes);
            let cached_data = code_cache_store.as_ref().and_then(|cache| {
                let cache = cache.borrow();
                let entry = cache.get(specifier.as_str())?;
                if entry.hash == computed_hash {
                    Some(Cow::Owned(entry.data.clone()))
                } else {
                    None
                }
            });

            let code_cache = code_cache_store.as_ref().map(|_| SourceCodeCacheInfo {
                hash: computed_hash,
                data: cached_data,
            });

            Ok(ModuleSource::new(
                module_type,
                ModuleSourceCode::Bytes(source_bytes.into_boxed_slice().into()),
                &specifier,
                code_cache,
            ))
        }))
    }

    fn code_cache_ready(
        &self,
        module_specifier: ModuleSpecifier,
        hash: u64,
        code_cache: &[u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
        let Some(store) = self.code_cache.as_ref().cloned() else {
            return Box::pin(async {});
        };
        let key = module_specifier.to_string();
        let value = ModuleCodeCacheEntry {
            hash,
            data: code_cache.to_vec(),
        };
        Box::pin(async move {
            store.borrow_mut().insert(key, value);
        })
    }

    fn purge_and_prevent_code_cache(&self, module_specifier: &str) {
        if let Some(store) = &self.code_cache {
            store.borrow_mut().remove(module_specifier);
        }
    }
}
