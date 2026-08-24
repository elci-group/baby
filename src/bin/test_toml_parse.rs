use std::fs;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: test_toml_parse <path>");
    let text = fs::read_to_string(&path).expect("read");
    let value: toml::Value = toml::from_str(&text).expect("parse");

    println!("Full structure:\n{:#?}\n", value);

    if let Some(bin) = value.get("bin") {
        println!("bin value:\n{:#?}\n", bin);
        println!("bin.as_array():\n{:#?}\n", bin.as_array());
    }
}
