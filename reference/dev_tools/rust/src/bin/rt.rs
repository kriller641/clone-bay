// For every *.bin in a directory: decode with blue-marshal, re-encode (keeping
// the checksum flag) and write *.rs, or write the error to *.rserr.
fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().map(|e| e != "bin").unwrap_or(true) { continue; }
        let data = std::fs::read(&p).unwrap();
        match blue_marshal::decode(&data) {
            Ok(d) => {
                let o = blue_marshal::EncodeOptions { checksum: d.had_crc, ..Default::default() };
                match blue_marshal::encode(&d.value, &o) {
                    Ok(b) => std::fs::write(p.with_extension("rs"), b).unwrap(),
                    Err(e) => std::fs::write(p.with_extension("rserr"), format!("encode: {e}")).unwrap(),
                }
            }
            Err(e) => std::fs::write(p.with_extension("rserr"), format!("{e}")).unwrap(),
        }
    }
}
