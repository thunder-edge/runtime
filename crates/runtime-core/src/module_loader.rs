use std::sync::Arc;

use base64::Engine;
use deno_core::{
    error::ModuleLoaderError, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse,
    ModuleLoader, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType, ResolutionKind,
};
use eszip::EszipV2;

/// Module loader that resolves modules from an eszip bundle.
pub struct EszipModuleLoader {
    eszip: Arc<EszipV2>,
    inline_source_maps: bool,
}

impl EszipModuleLoader {
    pub fn new(eszip: Arc<EszipV2>) -> Self {
        Self {
            eszip,
            inline_source_maps: true,
        }
    }

    pub fn new_with_source_maps(eszip: Arc<EszipV2>, inline_source_maps: bool) -> Self {
        Self {
            eszip,
            inline_source_maps,
        }
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

            Ok(ModuleSource::new(
                module_type,
                ModuleSourceCode::Bytes(source_bytes.into_boxed_slice().into()),
                &specifier,
                None,
            ))
        }))
    }
}
