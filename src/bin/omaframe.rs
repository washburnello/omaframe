//! omaframe headless export CLI (plan §5, for scripts/agents).
//!
//! ```text
//! omaframe --open FILE --export {txt|md|ansi} [--to OUT]
//!          [--selection x,y,w,h] [--copy]
//! ```
//!
//! Prints to stdout, or to `OUT` when `--to` is given. `--copy` additionally
//! pipes the output through [`omaframe::clipboard::copy_text`]
//! (`wl-copy`, else OSC52). Exits nonzero with stderr on errors (bad args,
//! unreadable file, bad geometry, IO/clipboard failures). Manual argv
//! parsing — no clap, no new dependencies.
//!
//! The testable core is [`run`]: it performs load/export/`--to` write/
//! `--copy` and returns the exported text, so `cargo test` can assert the
//! golden `testdata/demo.omaframe.json --export txt == testdata/demo.txt`
//! without spawning a subprocess.

use omaframe::clipboard;
use omaframe::model::{self, Rect};

/// CLI usage (returned as `Err` text for `--help` and arg errors; `main`
/// prints it to stderr with a nonzero exit).
fn usage() -> String {
    "usage: omaframe --open FILE --export {txt|md|ansi} [--to OUT] [--selection x,y,w,h] [--copy]"
        .to_string()
}

/// Parse `x,y,w,h` into a [`Rect`]. `x`/`y` may be negative (clipped at
/// export); `w`/`h` must be positive. Any malformed input is a loud
/// error (CLI exits nonzero) — never a silent clamp.
fn parse_selection(s: &str) -> Result<Rect, String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return Err(format!(
            "bad --selection {s:?} (want x,y,w,h, e.g. 6,0,8,1)"
        ));
    }
    let x: i32 = parts[0]
        .trim()
        .parse()
        .map_err(|_| format!("bad --selection {s:?} (x is not an integer)"))?;
    let y: i32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| format!("bad --selection {s:?} (y is not an integer)"))?;
    let w: i64 = parts[2]
        .trim()
        .parse()
        .map_err(|_| format!("bad --selection {s:?} (w is not an integer)"))?;
    let h: i64 = parts[3]
        .trim()
        .parse()
        .map_err(|_| format!("bad --selection {s:?} (h is not an integer)"))?;
    if w <= 0 || h <= 0 {
        return Err(format!(
            "bad --selection {s:?} (w and h must be >= 1)"
        ));
    }
    // Rect stores u32 sizes; i64 range check keeps the cast safe.
    if w > u32::MAX as i64 || h > u32::MAX as i64 {
        return Err(format!("bad --selection {s:?} (w/h too large)"));
    }
    Ok(Rect::new(x, y, w as u32, h as u32))
}

/// ANSI export clipped to `rect` (mirrors `model::export_ansi` row rules).
/// Unbounded like the infinite canvas (no grid clip); theme variables
/// concretize against `theme`.
fn export_ansi_selection(
    doc: &model::Document,
    rect: &Rect,
    theme: &omaframe::theme::Theme,
) -> String {
    use omaframe::model::PaintColor;
    type ColorSeg = (String, Option<(PaintColor, Option<PaintColor>)>);
    if rect.is_empty() {
        return String::new();
    }    let x_end = (rect.x as i64 + rect.w as i64).min(i32::MAX as i64);
    let y_end = (rect.y as i64 + rect.h as i64).min(i32::MAX as i64);
    let mut lines = Vec::new();
    for y in rect.y..y_end as i32 {
        let mut segs: Vec<ColorSeg> = Vec::new();
        for x in rect.x..x_end as i32 {
            if doc.is_continuation(x, y) {
                continue;
            }
            match doc.cell(x, y) {
                Some(c) => segs.push((c.ch.clone(), Some((c.fg, c.bg)))),
                None => segs.push((" ".to_string(), None)),
            }
        }
        while segs.last().is_some_and(|(s, col)| col.is_none() && s == " ") {
            segs.pop();
        }
        while segs.last().is_some_and(|(s, _)| s.is_empty()) {
            segs.pop();
        }
        if segs.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        let mut cur: Option<(PaintColor, Option<PaintColor>)> = None;
        for (s, col) in segs {
            if col != cur {
                match &col {
                    None => line.push_str("\x1b[0m"),
                    Some((fg, bg)) => {
                        line.push_str(&PaintColor::cell_escape(fg, bg, theme))
                    }
                }
                cur = col;
            }
            line.push_str(&s);
        }
        if cur.is_some() {
            line.push_str("\x1b[0m");
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

/// Headless export core: parse `args`, load, export, handle `--to`/`--copy`.
///
/// `args` is the full argv **including** the program name at `args[0]`
/// (as from `std::env::args()`); a slice without a program name (first
/// element starts with `-`) is also accepted for tests.
///
/// Returns the exported text on success (already written to `--to` path
/// and/or clipboard when those flags are present). Returns `Err(message)`
/// for any failure; `main` prints it to stderr and exits nonzero.
pub fn run(args: &[String]) -> Result<String, String> {
    let argv: &[String] = if args.is_empty() {
        return Err(usage());
    } else if args[0].starts_with('-') {
        args
    } else {
        &args[1..]
    };

    let mut open: Option<String> = None;
    let mut export: Option<String> = None;
    let mut to: Option<String> = None;
    let mut selection: Option<String> = None;
    let mut copy = false;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--open" => {
                i += 1;
                if i >= argv.len() {
                    return Err("missing value for --open".to_string());
                }
                open = Some(argv[i].clone());
            }
            "--export" => {
                i += 1;
                if i >= argv.len() {
                    return Err("missing value for --export (want txt|md|ansi)".to_string());
                }
                export = Some(argv[i].clone());
            }
            "--to" => {
                i += 1;
                if i >= argv.len() {
                    return Err("missing value for --to".to_string());
                }
                to = Some(argv[i].clone());
            }
            "--selection" => {
                i += 1;
                if i >= argv.len() {
                    return Err("missing value for --selection (want x,y,w,h)".to_string());
                }
                selection = Some(argv[i].clone());
            }
            "--copy" => {
                copy = true;
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            other => {
                return Err(format!("unknown argument {other:?}\n{}", usage()));
            }
        }
        i += 1;
    }

    let open_path = open.ok_or_else(usage)?;
    let export_fmt = export.ok_or_else(usage)?;
    if export_fmt != "txt" && export_fmt != "md" && export_fmt != "ansi" {
        return Err(format!(
            "bad --export {export_fmt:?} (want txt|md|ansi)"
        ));
    }

    let text = std::fs::read_to_string(&open_path)
        .map_err(|e| format!("cannot read {open_path}: {e}"))?;
    let doc = model::load_json(&text).map_err(|e| format!("cannot load {open_path}: {e}"))?;
    // ANSI export concretizes theme variables against the active theme.
    let theme = omaframe::theme::load();

    let output = if let Some(sel_str) = selection {
        let rect = parse_selection(&sel_str)?;
        match export_fmt.as_str() {
            "txt" => model::export_selection(&doc, &rect),
            "md" => format!("# {}\n```text\n{}```\n", doc.name, model::export_selection(&doc, &rect)),
            "ansi" => export_ansi_selection(&doc, &rect, &theme),
            _ => return Err(format!("bad --export {export_fmt:?} (want txt|md|ansi)")),
        }
    } else {
        match export_fmt.as_str() {
            "txt" => model::export_txt(&doc),
            "md" => model::export_md(&doc),
            "ansi" => model::export_ansi(&doc, &theme),
            _ => return Err(format!("bad --export {export_fmt:?} (want txt|md|ansi)")),
        }
    };

    if let Some(out_path) = to {
        std::fs::write(&out_path, &output)
            .map_err(|e| format!("cannot write {out_path}: {e}"))?;
    }
    if copy {
        clipboard::copy_text(&output).map_err(|e| format!("copy failed: {e}"))?;
    }
    Ok(output)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(out) => {
            // `run` already wrote `--to` files; print to stdout only for
            // the stdout path (spec: "prints to stdout or OUT file").
            let has_to = args.iter().any(|a| a == "--to");
            if !has_to {
                print!("{out}");
            }
        }
        Err(e) => {
            eprintln!("omaframe: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use std::path::PathBuf;

    fn manifest_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn demo_json() -> String {
        manifest_dir()
            .join("testdata/demo.omaframe.json")
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn golden_demo_txt_is_byte_identical() {
        let expected =
            std::fs::read_to_string(manifest_dir().join("testdata/demo.txt")).expect("read demo.txt");
        let args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "txt".to_string(),
        ];
        let out = run(&args).expect("run txt");
        assert_eq!(out, expected);
    }

    #[test]
    fn export_md_wraps_txt() {
        let args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "md".to_string(),
        ];
        let out = run(&args).expect("run md");
        assert!(out.starts_with("# settings-panel\n```text\n"));
        assert!(out.ends_with("```\n"));
        assert!(out.contains("Settings"));
    }

    #[test]
    fn export_ansi_contains_sgr_and_strips_to_txt() {
        let txt_args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "txt".to_string(),
        ];
        let txt = run(&txt_args).expect("run txt");
        let ansi_args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "ansi".to_string(),
        ];
        let ansi = run(&ansi_args).expect("run ansi");
        assert!(ansi.contains("\x1b["), "expected SGR codes");
        // Strip SGR; remainder must equal plain txt.
        let mut stripped = String::new();
        let mut chars = ansi.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                assert_eq!(chars.next(), Some('['));
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                stripped.push(c);
            }
        }
        assert_eq!(stripped, txt);
    }

    #[test]
    fn selection_exports_title_span() {
        let args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "txt".to_string(),
            "--selection".to_string(),
            "6,0,8,1".to_string(),
        ];
        assert_eq!(run(&args).expect("run selection"), "Settings\n");
    }

    #[test]
    fn to_flag_writes_file() {
        let out_path = std::env::temp_dir().join(format!("omaframe-test-{}.txt", std::process::id()));
        let args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "txt".to_string(),
            "--to".to_string(),
            out_path.to_string_lossy().to_string(),
        ];
        let out = run(&args).expect("run --to");
        let back = std::fs::read_to_string(&out_path).expect("read back --to");
        assert_eq!(out, back);
        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn bad_args_fail_loudly() {
        // Missing --open.
        let args = vec!["omaframe".to_string(), "--export".to_string(), "txt".to_string()];
        assert!(run(&args).is_err());
        // Bad export kind.
        let args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "pdf".to_string(),
        ];
        assert!(run(&args).is_err());
        // Malformed selection.
        let args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            demo_json(),
            "--export".to_string(),
            "txt".to_string(),
            "--selection".to_string(),
            "1,2,0,5".to_string(),
        ];
        assert!(run(&args).is_err());
        // Missing file.
        let args = vec![
            "omaframe".to_string(),
            "--open".to_string(),
            "/nonexistent/omaframe-missing.json".to_string(),
            "--export".to_string(),
            "txt".to_string(),
        ];
        assert!(run(&args).is_err());
        // Unknown flag.
        let args = vec!["omaframe".to_string(), "--frobnicate".to_string()];
        assert!(run(&args).is_err());
    }
}
