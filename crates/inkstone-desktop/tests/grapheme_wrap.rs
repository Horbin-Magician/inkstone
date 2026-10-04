// Compile the exact helper used by the controlled gpui-base patch, so the
// standard application test command checks it without upstream benchmark deps.
#[path = "../../../vendor/gpui-base/src/input/editor/display_map/grapheme_wrap.rs"]
mod grapheme_wrap;
