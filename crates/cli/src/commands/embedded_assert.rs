use deno_graph::source::LoadError;

const ASSERT_USER_MOD_TS: &str = include_str!("../../../runtime-core/src/assert/user_mod.ts");
const ASSERT_MOD_TS: &str = include_str!("../../../runtime-core/src/assert/mod.ts");
const ASSERT_ASSERT_TS: &str = include_str!("../../../runtime-core/src/assert/assert.ts");
const ASSERT_MOCK_MOD_TS: &str = include_str!("../../../runtime-core/src/assert/mock/mod.ts");
const ASSERT_MOCK_FN_TS: &str = include_str!("../../../runtime-core/src/assert/mock/mockFn.ts");
const ASSERT_MOCK_SPY_TS: &str = include_str!("../../../runtime-core/src/assert/mock/spy.ts");
const ASSERT_MOCK_FETCH_TS: &str = include_str!("../../../runtime-core/src/assert/mock/fetch.ts");
const ASSERT_MOCK_TIME_TS: &str = include_str!("../../../runtime-core/src/assert/mock/time.ts");
const HTTP_USER_MOD_TS: &str = include_str!("../../../runtime-core/src/http/user_mod.ts");
const HTTP_MOD_TS: &str = include_str!("../../../runtime-core/src/http/mod.ts");
const HTTP_HTTP_TS: &str = include_str!("../../../runtime-core/src/http/http.ts");

const THUNDER_TESTING_ALIAS: &str = "thunder:testing";
const THUNDER_HTTP_ALIAS: &str = "thunder:http";
const EDGE_ASSERT_MOD_SPECIFIER: &str = "edge://assert/mod.ts";
const EDGE_HTTP_MOD_SPECIFIER: &str = "edge://http/mod.ts";

pub fn rewrite_edge_assert_imports(content: Vec<u8>) -> Vec<u8> {
    // Map local CLI shorthand aliases to embedded modules.
    String::from_utf8_lossy(&content)
        .replace(THUNDER_TESTING_ALIAS, EDGE_ASSERT_MOD_SPECIFIER)
        .replace(THUNDER_HTTP_ALIAS, EDGE_HTTP_MOD_SPECIFIER)
        .into_bytes()
}

pub fn load_module_bytes(
    specifier: &deno_graph::ModuleSpecifier,
) -> Result<Option<Vec<u8>>, LoadError> {
    let spec = specifier.as_str();
    let source = match spec {
        "edge://assert/mod.ts" => ASSERT_USER_MOD_TS,
        "edge://assert/assert.ts" => ASSERT_ASSERT_TS,
        "edge://assert/mock/mod.ts" => ASSERT_MOCK_MOD_TS,
        "edge://assert/mock/mockFn.ts" => ASSERT_MOCK_FN_TS,
        "edge://assert/mock/spy.ts" => ASSERT_MOCK_SPY_TS,
        "edge://assert/mock/fetch.ts" => ASSERT_MOCK_FETCH_TS,
        "edge://assert/mock/time.ts" => ASSERT_MOCK_TIME_TS,
        "edge://http/mod.ts" => HTTP_USER_MOD_TS,
        "edge://http/http.ts" => HTTP_HTTP_TS,
        "ext:edge_assert/mod.ts" => ASSERT_MOD_TS,
        "ext:edge_http/mod.ts" => HTTP_MOD_TS,
        "ext:edge_http/http.ts" => HTTP_HTTP_TS,
        "ext:edge_assert/assert.ts" => {
            return Ok(Some(
                ASSERT_ASSERT_TS
                    .replace("\"./mock/mod.ts\"", "\"edge://assert/mock/mod.ts\"")
                    .into_bytes(),
            ));
        }
        "ext:edge_assert/mock/mod.ts" => {
            return Ok(Some(
                ASSERT_MOCK_MOD_TS
                    .replace("\"./mockFn.ts\"", "\"edge://assert/mock/mockFn.ts\"")
                    .replace("\"./spy.ts\"", "\"edge://assert/mock/spy.ts\"")
                    .replace("\"./fetch.ts\"", "\"edge://assert/mock/fetch.ts\"")
                    .replace("\"./time.ts\"", "\"edge://assert/mock/time.ts\"")
                    .into_bytes(),
            ));
        }
        "ext:edge_assert/mock/mockFn.ts" => ASSERT_MOCK_FN_TS,
        "ext:edge_assert/mock/spy.ts" => {
            return Ok(Some(
                ASSERT_MOCK_SPY_TS
                    .replace("\"./mockFn.ts\"", "\"edge://assert/mock/mockFn.ts\"")
                    .into_bytes(),
            ));
        }
        "ext:edge_assert/mock/fetch.ts" => {
            return Ok(Some(
                ASSERT_MOCK_FETCH_TS
                    .replace("\"./mockFn.ts\"", "\"edge://assert/mock/mockFn.ts\"")
                    .into_bytes(),
            ));
        }
        "ext:edge_assert/mock/time.ts" => ASSERT_MOCK_TIME_TS,
        _ => return Ok(None),
    };

    Ok(Some(source.as_bytes().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_edge_assert_imports_unchanged() {
        let input = b"import { assertEquals } from 'edge://assert/mod.ts';\n".to_vec();
        let output = rewrite_edge_assert_imports(input);
        let out = String::from_utf8(output).expect("utf8");

        assert!(out.contains("edge://assert/mod.ts"));
    }

    #[test]
    fn rewrites_thunder_testing_alias() {
        let input = b"import { assertEquals } from 'thunder:testing';\n".to_vec();
        let output = rewrite_edge_assert_imports(input);
        let out = String::from_utf8(output).expect("utf8");

        assert!(out.contains("edge://assert/mod.ts"));
        assert!(!out.contains("thunder:testing"));
    }

    #[test]
    fn rewrites_thunder_http_alias() {
        let input = b"import { JSONResponse } from 'thunder:http';\n".to_vec();
        let output = rewrite_edge_assert_imports(input);
        let out = String::from_utf8(output).expect("utf8");

        assert!(out.contains("edge://http/mod.ts"));
        assert!(!out.contains("thunder:http"));
    }

    #[test]
    fn provides_embedded_ext_module() {
        let specifier =
            deno_graph::ModuleSpecifier::parse("ext:edge_assert/assert.ts").expect("specifier");
        let maybe = load_module_bytes(&specifier).expect("load");
        let bytes = maybe.expect("module must exist");
        let source = String::from_utf8(bytes).expect("utf8");

        assert!(source.contains("export class AssertionError"));
        assert!(source.contains("edge://assert/mock/mod.ts"));
    }

    #[test]
    fn provides_embedded_http_module() {
        let specifier = deno_graph::ModuleSpecifier::parse("edge://http/mod.ts").expect("specifier");
        let maybe = load_module_bytes(&specifier).expect("load");
        let bytes = maybe.expect("module must exist");
        let source = String::from_utf8(bytes).expect("utf8");

        assert!(source.contains("HTTP"));
        assert!(source.contains("./http.ts"));
    }
}
