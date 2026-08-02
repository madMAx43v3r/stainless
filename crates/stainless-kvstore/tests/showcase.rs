use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn stainless_store_commits_reverts_recovers_and_reads_concurrently() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock follows Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "stainless-kvstore-{}-{unique}.log",
        std::process::id()
    ));

    stainless_kvstore::self_test(&path.to_string_lossy())
        .unwrap_or_else(|error| panic!("showcase failed: {error}"));
}
