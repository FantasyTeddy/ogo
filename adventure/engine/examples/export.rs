//! Writes the sample world as an .adv file: `cargo run --example export -- sample.adv`.

fn main() -> std::io::Result<()> {
    let path = std::env::args().nth(1).expect("usage: export <file.adv>");
    std::fs::write(&path, adventure_engine::sample::world().to_bytes())
}
