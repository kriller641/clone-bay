fuzz.py imports the codec as "codec": put a copy of eanm_sync.py next to it named codec.py.
e2e.py imports the script as eanm_sync (from the script's own folder).
Build Rust helpers: cd rust && cargo build --release  (binaries: target/release/rt, target/release/tojson)
