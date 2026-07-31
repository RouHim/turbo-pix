use std::{env, fs, path::Path};

const TEXT_EXTS: &[&str] = &[
    "html",
    "js",
    "css",
    "svg",
    "json",
    "txt",
    "map",
    "webmanifest",
];

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let dist = Path::new(&manifest_dir).join("dist");

    if !dist.join("index.html").exists() {
        panic!("dist/index.html not found — run `npm run build` before `cargo build`");
    }

    let mut text_entries = Vec::new();
    let mut binary_entries = Vec::new();
    collect(&dist, &dist, &mut text_entries, &mut binary_entries);

    let mut code = String::from("const STATIC_FILES: &[(&str, &str)] = &[\n");
    for (rel, abs) in &text_entries {
        code.push_str(&format!("    ({rel:?}, include_str!({abs:?})),\n"));
    }
    code.push_str("];\n\nconst STATIC_BINARY_FILES: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &binary_entries {
        code.push_str(&format!(
            "    ({rel:?}, include_bytes!({abs:?}) as &[u8]),\n"
        ));
    }
    code.push_str("];\n");

    let out_dir = env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out_dir).join("embedded_static.rs"), code).unwrap();
}

fn collect(
    dir: &Path,
    root: &Path,
    text: &mut Vec<(String, String)>,
    binary: &mut Vec<(String, String)>,
) {
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, root, text, binary);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let abs = path.to_string_lossy().replace('\\', "/");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if TEXT_EXTS.contains(&ext) {
                text.push((rel, abs));
            } else {
                binary.push((rel, abs));
            }
        }
    }
}
