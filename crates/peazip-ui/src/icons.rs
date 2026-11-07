// Original PeaZip 11.4.0 main/main-dark theme PNGs, LGPL-3.0-or-later.
// Stored as data URIs without pixel changes so every host uses the same bundled assets.
// The generic file sheet is a code-drawn fallback; file associations are not migrated yet.
use nsh_core::prelude::*;
pub(crate) fn icon(name: &'static str) -> impl IntoElement {
    image(move || if name == "file" {
        nsh_core::element::ImageSrc::svg_markup("<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'><path d='M3 1.5h7l3 3v10H3z M9.5 1.5V5H13 M5 8h6 M5 10h6 M5 12h4' fill='none' stroke='#888'/></svg>")
    } else { match (name, dark()) {
        ("folder", false) => include_str!("../assets/folder-main.uri"),
        ("folder", true) => include_str!("../assets/folder-main-dark.uri"),
        ("back", false) => include_str!("../assets/back-main.uri"),
        ("back", true) => include_str!("../assets/back-main-dark.uri"),
        ("forward", false) => include_str!("../assets/forward-main.uri"),
        ("forward", true) => include_str!("../assets/forward-main-dark.uri"),
        ("up", false) => include_str!("../assets/up-main.uri"),
        ("up", true) => include_str!("../assets/up-main-dark.uri"),
        ("add", false) => include_str!("../assets/add-main.uri"),
        ("add", true) => include_str!("../assets/add-main-dark.uri"),
        ("extract", false) => include_str!("../assets/extract-main.uri"),
        ("extract", true) => include_str!("../assets/extract-main-dark.uri"),
        ("test", false) => include_str!("../assets/test-main.uri"),
        ("test", true) => include_str!("../assets/test-main-dark.uri"),
        ("open", false) => include_str!("../assets/open-main.uri"),
        ("open", true) => include_str!("../assets/open-main-dark.uri"),
        ("save", false) => include_str!("../assets/save-main.uri"),
        ("save", true) => include_str!("../assets/save-main-dark.uri"),
        _ => "",
    }.into() }).w(px(16.0)).h(px(16.0))
}
