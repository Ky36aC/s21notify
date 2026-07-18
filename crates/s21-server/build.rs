// rust-embed требует, чтобы папка miniapp/dist существовала на этапе компиляции.
// В обычной сборке (cargo check/test без trunk) её нет — создаём пустую, чтобы
// не падать. Реальные ассеты кладёт `trunk build` перед релизной сборкой; тогда
// они и встроятся в бинарь.
fn main() {
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let dist = std::path::Path::new(&manifest).join("../../miniapp/dist");
        let _ = std::fs::create_dir_all(&dist);
    }
    println!("cargo:rerun-if-changed=../../miniapp/dist");
}
