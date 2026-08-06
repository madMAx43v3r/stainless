use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_log(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock follows Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "stainless-kvstore-{label}-{}-{unique}.log",
        std::process::id()
    ))
}

#[test]
fn stainless_store_end_to_end() {
    let path = temporary_log("showcase");
    stainless_kvstore::self_test(&path.to_string_lossy())
        .unwrap_or_else(|error| panic!("showcase failed: {error}"));
}
