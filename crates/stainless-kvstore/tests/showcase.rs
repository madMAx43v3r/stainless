use std::time::{SystemTime, UNIX_EPOCH};

use stainless_kvstore::{Codec, Table};

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
fn stainless_store_commits_reverts_recovers_and_reads_concurrently() {
    let path = temporary_log("raw");

    stainless_kvstore::self_test(&path.to_string_lossy())
        .unwrap_or_else(|error| panic!("showcase failed: {error}"));
}

#[test]
fn typed_store_persists_compound_keys_and_generic_values() {
    let path = temporary_log("typed");
    let table = Table::<(u32, String), String>::open(&path)
        .unwrap_or_else(|error| panic!("open failed: {error}"));
    let key = (7, "alice".to_owned());

    table
        .insert(key.clone(), "first".to_owned())
        .unwrap_or_else(|error| panic!("insert failed: {error}"));
    assert!(table.commit(1).expect("first commit should succeed"));
    table
        .insert(key.clone(), "second".to_owned())
        .unwrap_or_else(|error| panic!("second insert failed: {error}"));
    assert!(table.commit(2).expect("second commit should succeed"));
    assert_eq!(
        table.find(&key).expect("find should succeed"),
        Some("second".to_owned())
    );
    assert!(table.revert(1).expect("revert should succeed"));
    drop(table);

    let recovered = Table::<(u32, String), String>::open(&path)
        .unwrap_or_else(|error| panic!("recovery failed: {error}"));
    assert_eq!(recovered.current_version(), 1);
    assert_eq!(
        recovered.find(&key).expect("recovered find should succeed"),
        Some("first".to_owned())
    );
    std::fs::remove_file(path).expect("typed showcase log should be removable");
}

#[test]
fn compound_key_codec_preserves_tuple_order() {
    let mut keys = [
        (2_u32, "alpha".to_owned()),
        (1_u32, "zeta".to_owned()),
        (1_u32, "alpha".to_owned()),
    ];
    keys.sort();
    let mut encoded = keys
        .iter()
        .map(|key| key.encode().expect("compound key should encode"))
        .collect::<Vec<_>>();
    let already_sorted = encoded.clone();
    encoded.sort();
    assert_eq!(encoded, already_sorted);
}

#[test]
fn wal_integer_fields_use_fixed_width_big_endian_encoding() {
    let path = temporary_log("big-endian");
    let table =
        Table::<u32, u32>::open(&path).unwrap_or_else(|error| panic!("open failed: {error}"));
    table
        .insert(0x0102_0304, 0x0506_0708)
        .unwrap_or_else(|error| panic!("insert failed: {error}"));
    assert!(table.commit(1).expect("commit should succeed"));
    drop(table);

    let bytes = std::fs::read(&path).expect("WAL should be readable");
    assert_eq!(bytes.len(), 37 + 29);
    assert_eq!(&bytes[0..8], &37_u64.to_be_bytes());
    assert_eq!(bytes[8], 0);
    assert_eq!(&bytes[9..13], &0_u32.to_be_bytes());
    assert_eq!(&bytes[13..21], &4_u64.to_be_bytes());
    assert_eq!(&bytes[21..25], &4_u32.to_be_bytes());
    assert_eq!(&bytes[25..29], &0x0102_0304_u32.to_be_bytes());
    assert_eq!(&bytes[29..33], &0x0506_0708_u32.to_be_bytes());

    assert_eq!(&bytes[37..45], &29_u64.to_be_bytes());
    assert_eq!(bytes[45], 1);
    assert_eq!(&bytes[46..50], &1_u32.to_be_bytes());
    assert_eq!(&bytes[50..58], &0_u64.to_be_bytes());
    assert_eq!(&bytes[58..62], &0_u32.to_be_bytes());

    std::fs::remove_file(path).expect("big-endian WAL should be removable");
}
