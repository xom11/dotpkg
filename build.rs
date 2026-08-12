//! Embed an `asInvoker` manifest in the test binaries, on Windows only.
//!
//! # The measurement this exists for
//!
//! `tests/update.rs` compiles to `update-<hash>.exe`. Windows' UAC installer
//! detection inspects an executable's **filename** for `install`, `setup`,
//! `update` or `patch` and, for a binary that carries no manifest, decides it
//! is an installer and requires elevation. From an ordinary (non-elevated)
//! session nothing can start it -- cargo reports
//!
//! ```text
//! could not execute process ...\update-<hash>.exe (never executed)
//! Caused by: The requested operation requires elevation. (os error 740)
//! ```
//!
//! **Measured with a control**, not reasoned: one binary copied to three names
//! and each launch verified by its *output* rather than its exit code (a
//! libtest binary asked to `--list` prints one line per test).
//!
//! | name | keyword | test names printed |
//! |---|---|---|
//! | `update-<hash>.exe` | `update` | **0** |
//! | `ph6-neutral-probe.exe` | none | **24** |
//! | `ph6-setup-probe.exe` | `setup` | **0** |
//!
//! Same bytes, same sha256; the name is the only variable, and a *different*
//! keyword fails the same way -- so this is a class, not one file. See
//! `docs/measurements-2026-08-12-phase6-citations.md` §13a, and note that the
//! same document records an earlier probe that appeared to refute all this
//! because it read an exit code instead of the output.
//!
//! # Why a manifest rather than renaming the test file
//!
//! Renaming closes one instance; a future `tests/setup.rs` reopens it. A
//! manifest suppresses installer detection outright -- the heuristic applies
//! only to binaries that do not declare a level -- so it closes the class for
//! every test target this crate will ever have. It also leaves the record
//! alone: about ten places in `docs/` name `tests/update.rs`, and they are true
//! about the trees they were written against.
//!
//! # Why this touches nothing that ships
//!
//! `cargo::rustc-link-arg-tests` applies to **test targets only**. The
//! `dotpkg` binary and the library are not affected, and neither is any
//! non-Windows build: everything below is inside a `target_os = "windows"`
//! check, and the linker flags are MSVC's.

use std::io::Write;

fn main() {
    // Re-run only when this file changes. Without this, cargo re-runs the
    // script whenever any file in the package changes, which is the default
    // and is wasteful for a script that reads nothing.
    println!("cargo::rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // MSVC's linker embeds the manifest itself; the GNU toolchain would need a
    // different mechanism, and this project builds Windows with MSVC.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR");
    let manifest_path = std::path::Path::new(&out_dir).join("as-invoker.manifest");

    // `level="asInvoker" uiAccess="false"` is the "run with whatever token I
    // was given" declaration. Its presence is the whole point: installer
    // detection is skipped for any executable that states a level at all.
    let manifest = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

    let mut file = std::fs::File::create(&manifest_path)
        .unwrap_or_else(|e| panic!("could not write {}: {e}", manifest_path.display()));
    file.write_all(manifest.as_bytes())
        .unwrap_or_else(|e| panic!("could not write {}: {e}", manifest_path.display()));

    println!("cargo::rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo::rustc-link-arg-tests=/MANIFESTINPUT:{}",
        manifest_path.display()
    );
}
