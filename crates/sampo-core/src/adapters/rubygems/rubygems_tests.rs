use super::*;

#[test]
fn version_exists_rejects_empty_name() {
    let err = RubyGemsAdapter
        .version_exists("", "1.0.0", None)
        .expect_err("expected empty package name to fail");
    assert!(format!("{}", err).contains("Package name cannot be empty"));
}

#[test]
fn version_exists_defers_to_the_private_push_host() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("private.gemspec");
    std::fs::write(
        &manifest,
        "Gem::Specification.new do |spec|\n  spec.name = \"private\"\n  \
         spec.version = \"1.0.0\"\n  \
         spec.metadata[\"allowed_push_host\"] = \"https://gems.example.com\"\nend\n",
    )
    .unwrap();

    let exists = RubyGemsAdapter
        .version_exists("private", "1.0.0", Some(manifest.as_path()))
        .expect("a private push host should defer without erroring");
    assert!(!exists);
}
