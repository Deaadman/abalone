//! Draws the pictures in `docs/images` from the examples' real screens, as
//! SVG. Ignored by `cargo test`; after changing something the docs show, run
//!
//! `cargo test --all-features --test docs_pictures -- --ignored`
//!
//! and look over the changed pictures before committing. CI checks they're current.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use shellkit::out::Kind;
use shellkit::status::UpdateNote;
use shellkit::{Color, Item, KeyCode, KeyEvent, KeyModifiers, Notice, Row, Shell, Tui, update};

#[allow(dead_code)]
#[path = "../examples/minimal.rs"]
mod minimal;

#[allow(dead_code)]
#[path = "../examples/custom.rs"]
mod custom;

/// Every picture is this many columns wide.
const W: usize = 100;
const H: usize = 26;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn typed<A: shellkit::App>(shell: &mut Shell<A>, text: &str) {
    for c in text.chars() {
        shell.key(key(KeyCode::Char(c)));
    }
}

fn screen<A: shellkit::App>(shell: &mut Shell<A>) -> Vec<Row> {
    shell.render(W, H).rows
}

fn save(name: &str, rows: &[Row]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs").join("images");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), svg(rows)).unwrap();
}

/// Rows as an SVG: each run of text placed on the character grid, so it
/// lines up whatever monospace font the viewer has. Blank rows at the
/// bottom are left off.
fn svg(rows: &[Row]) -> String {
    const CW: f64 = 8.4;
    const LH: usize = 18;
    const PAD: usize = 16;
    let hex = |c: Option<Color>, default: &str| match c {
        Some(Color::Rgb { r, g, b }) => format!("#{r:02x}{g:02x}{b:02x}"),
        Some(Color::Black) => "#000000".into(),
        Some(Color::White) => "#ffffff".into(),
        _ => default.into(),
    };
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut rows: Vec<&Row> = rows.iter().collect();
    while rows.last().is_some_and(|r| r.iter().all(|(_, t)| t.trim().is_empty())) {
        rows.pop();
    }
    let (w, h) = ((W as f64 * CW) as usize + 2 * PAD, rows.len() * LH + 2 * PAD);
    let mut out = vec![
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="Cascadia Mono, Consolas, Menlo, monospace" font-size="14">"#
        ),
        format!(r##"<rect width="{w}" height="{h}" rx="8" fill="#16161e"/>"##),
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = PAD + r * LH;
        // Clipped like the shell: one short of the width.
        let mut col = 0;
        for (style, text) in row.iter() {
            let text: String = text.chars().take((W - 1).saturating_sub(col)).collect();
            let n = text.chars().count();
            let x = PAD as f64 + col as f64 * CW;
            if let Some(bg) = style.bg {
                out.push(format!(
                    r#"<rect x="{x:.1}" y="{y}" width="{:.1}" height="{LH}" fill="{}"/>"#,
                    n as f64 * CW,
                    hex(Some(bg), "")
                ));
            }
            if !text.trim().is_empty() {
                let mut attrs = format!(r#"x="{x:.1}" y="{}" fill="{}""#, y + LH - 5, hex(style.fg, "#d8d8e0"));
                if style.bold {
                    attrs.push_str(r#" font-weight="bold""#);
                }
                if n > 1 {
                    attrs.push_str(&format!(
                        r#" textLength="{:.1}" lengthAdjust="spacingAndGlyphs""#,
                        n as f64 * CW
                    ));
                }
                out.push(format!(r#"<text {attrs} xml:space="preserve">{}</text>"#, esc(&text)));
            }
            col += n;
        }
    }
    out.push("</svg>".into());
    out.join("\n") + "\n"
}

#[test]
#[ignore = "writes docs/images; run it by name"]
fn draw_docs_pictures() {
    // The minimal example first: the custom one changes the theme for good.
    let mut shell = minimal::shell();
    shell.set_logo_frame(99);
    save("minimal.svg", &screen(&mut shell));
    shell.submit("/greet --name Sam");
    shell.transcript_mut().push(Item::Line(Kind::Out, "Hello, Sam!".into()));
    save("minimal-greeted.svg", &screen(&mut shell));

    let mut shell = custom::shell(Tui::Fullscreen);
    shell.set_logo_frame(99);
    shell.submit("/pick apple");
    shell
        .transcript_mut()
        .push(Item::Line(Kind::Out, "Picked apple".into()));
    // Drawn as running, rather than run.
    shell.transcript_mut().push(Item::Echo("/wait".into()));
    shell.pretend_job(Some(("Waiting".into(), Instant::now() - Duration::from_secs(3))));
    save("custom.svg", &screen(&mut shell));
    shell.pretend_job(None);

    shell.reset_view();
    typed(&mut shell, "/");
    save("menu.svg", &screen(&mut shell));
    shell.reset_view();
    typed(&mut shell, "/pick ");
    save("menu-values.svg", &screen(&mut shell));

    shell.reset_view();
    shell.open_help();
    save("help.svg", &screen(&mut shell));
    shell.key(key(KeyCode::Right));
    save("help-commands.svg", &screen(&mut shell));

    shell.reset_view();
    shell.open_settings();
    save("settings.svg", &screen(&mut shell));
    shell.key(key(KeyCode::Right));
    save("settings-change.svg", &screen(&mut shell));

    shell.reset_view();
    shell.open_release_notes();
    save("release-notes.svg", &screen(&mut shell));
    shell.key(key(KeyCode::Down));
    shell.key(key(KeyCode::Enter));
    save("release-notes-shown.svg", &screen(&mut shell));

    shell.reset_view();
    shell.submit("/list");
    save("panel.svg", &screen(&mut shell));

    // The status bar alone: a running command, an update, a warning.
    shell.reset_view();
    shell.set_notices(vec![
        UpdateNote::Installed.notice("/update"),
        Notice::warn("No fruit picked · /pick one"),
    ]);
    shell.pretend_job(Some(("Waiting".into(), Instant::now() - Duration::from_secs(3))));
    let rows = screen(&mut shell);
    save("status-bar.svg", &rows[rows.len() - 5..]);
    shell.pretend_job(None);

    // What an update says on its first launch.
    let news = update::news(
        "Custom demo updated to v0.2.0",
        &[
            "Release notes, from the changelog built into the app.".into(),
            "/tui default, to run in the terminal's own scrollback.".into(),
        ],
        "github.com/me/demo/releases/tag/0.2.0",
    );
    let transcript = shell.transcript_mut();
    transcript.clear();
    transcript.extend(news.into_iter().map(Item::Row));
    save("whats-new.svg", &shell.transcript_rows(W));
}
