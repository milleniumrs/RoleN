//! Cross-instance keychain roundtrip: set with one Entry, get with another,
//! for various key shapes. Run: cargo test -p rolen-core --test keyring_debug -- --nocapture

use rolen_core::secrets;

#[test]
fn cross_instance_roundtrip() {
    for key in [
        "testprobe",
        "__rolen_probe__",
        "test-hyphen-key",
        "test:colon-key",
    ] {
        let set_res = secrets::set_secret(key, "ok");
        let get_res = secrets::get_secret(key);
        println!(
            "{key:<20} set={} get={}",
            set_res.is_ok(),
            match &get_res {
                Ok(v) => format!("Ok({v:?})"),
                Err(e) => format!("Err({e})"),
            }
        );
        let _ = secrets::delete_secret(key);
    }
}
