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
