//! Check a screen bundle or a patch the way the device does, for server authors:
//!
//! ```sh
//! cargo run -p zectrix-screen --example n4s-check -- screen.n4s [out.pbm]
//! cargo run -p zectrix-screen --example n4s-check -- reply.n4p
//! ```
//!
//! A file that starts with `N4S1` is a bundle, anything else a patch. For a bundle, the
//! optional second argument writes the screen as shown (selected row highlighted) as a
//! PBM image.

use std::process::ExitCode;

use zectrix_screen::{Layout, MonoFrame, Patch, Screen};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: n4s-check <file.n4s|file.n4p> [out.pbm]");
        return ExitCode::from(2);
    };
    let mut body = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::from(2);
        }
    };
    if !body.starts_with(b"N4S1") {
        return match Patch::parse(&body) {
            Ok(p) => {
                let r = p.rect();
                println!(
                    "patch {}x{} at ({}, {}), {} B, highlighted plane: {}",
                    r.w,
                    r.h,
                    r.x,
                    r.y,
                    body.len(),
                    if p.has_highlighted() { "yes" } else { "no" }
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("bad patch: {e:?}");
                ExitCode::FAILURE
            }
        };
    }
    let layout = match Layout::parse(&body) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bad bundle: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "bundle {} B, {} rows, header: {}, selected: {}",
        layout.len(),
        layout.n_rows(),
        layout.has_header(),
        layout.initial_selected()
    );
    for i in 0..layout.n_rows() {
        let r = layout.row_rect(i).unwrap();
        let kind = if layout.selectable().contains(&i) {
            "row"
        } else {
            "header"
        };
        println!(
            "  {i:2} {kind:6} x {:3} y {:3} w {:3} h {:3}",
            r.x, r.y, r.w, r.h
        );
    }
    if let Some(out) = args.get(1) {
        let screen = Screen::new(&mut body).unwrap();
        let mut frame = MonoFrame::new();
        screen.compose(&mut frame);
        // PBM: 1 = black, the frame: 1 = white.
        let mut pbm = b"P4\n400 300\n".to_vec();
        pbm.extend(frame.as_bytes().iter().map(|b| !b));
        if let Err(e) = std::fs::write(out, pbm) {
            eprintln!("{out}: {e}");
            return ExitCode::from(2);
        }
        println!("wrote {out}");
    }
    ExitCode::SUCCESS
}
