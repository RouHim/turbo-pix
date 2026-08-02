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

    // Stale-dist guard: if any frontend source is newer than the bundled
    // index.html, the embedded bundle is out of date. Emitting
    // rerun-if-changed for the sources makes cargo re-run this script
    // whenever they change, so the panic fires instead of silently shipping
    // a stale frontend.
    let frontend_root = Path::new(&manifest_dir).join("frontend");
    let mut newest_frontend = None;
    for source in [
        frontend_root.join("index.html"),
        frontend_root.join("src"),
        frontend_root.join("public"),
    ] {
        emit_rerun_and_track(&source, &mut newest_frontend);
    }
    if let (Some(src_mtime), Ok(dist_mtime)) = (
        newest_frontend,
        dist.join("index.html")
            .metadata()
            .and_then(|m| m.modified()),
    ) {
        if src_mtime > dist_mtime {
            panic!(
                "stale dist/ (frontend sources are newer than dist/index.html) — run `npm run build` before `cargo build`"
            );
        }
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

fn emit_rerun_and_track(path: &Path, newest: &mut Option<std::time::SystemTime>) {
    println!("cargo:rerun-if-changed={}", path.display());
    if path.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                emit_rerun_and_track(&entry.path(), newest);
            }
        }
    } else if let Ok(meta) = path.metadata() {
        if let Ok(mtime) = meta.modified() {
            *newest = Some(match *newest {
                Some(n) => n.max(mtime),
                None => mtime,
            });
        }
    }
}

fn collect(
    dir: &Path,
    root: &Path,
    text: &mut Vec<(String, String)>,
    binary: &mut Vec<(String, String)>,
) {
    println!("cargo:rerun-if-changed={}", dir.display());
    // Sort entries so the generated STATIC_FILES/STATIC_BINARY_FILES order is
    // deterministic (fs::read_dir order is filesystem-dependent).
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
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
