//! Product display names follow the active UI locale.
pub fn name() -> &'static str {
    name_for_locale(&gpui_component::locale())
}

fn name_for_locale(locale: &str) -> &'static str {
    let language = locale.split(['-', '_']).next().unwrap_or_default();
    if language.eq_ignore_ascii_case("zh") {
        "InkStone"
    } else {
        "墨砚"
    }
}
