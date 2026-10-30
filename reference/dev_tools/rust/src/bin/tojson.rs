// Independent check: decode with the blue-marshal crate and print lossless JSON.
fn main() {
    let p = std::env::args().nth(1).expect("file");
    let d = blue_marshal::decode(&std::fs::read(p).unwrap()).unwrap();
    println!("{}", serde_json::to_string(&blue_marshal::to_json(&d.value)).unwrap());
}
