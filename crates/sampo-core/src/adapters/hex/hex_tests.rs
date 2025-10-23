use super::*;
#[test]
fn version_exists_rejects_empty_name() {
    let err = HexAdapter
        .version_exists("", "1.0.0", None)
        .expect_err("expected empty package name to fail");
    assert!(format!("{}", err).contains("Package name cannot be empty"));
}
